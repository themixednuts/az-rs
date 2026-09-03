//! Vegetation region distribution asset format.

use std::borrow::Cow;
use std::io::{Cursor, Read, Write};
use std::ops::Range;

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::math::{Quat, UVec2};
use bevy::prelude::*;
use thiserror::Error;

/// Current vegetation distribution asset schema version.
pub const VEGETATION_DISTRIBUTION_ASSET_VERSION: u32 = 1;

/// File extensions claimed by [`VegetationDistributionAssetLoader`].
///
/// Vegetation distribution products keep their pak source extension
/// (`.distribution`).
pub const VEGETATION_DISTRIBUTION_ASSET_EXTENSIONS: &[&str] = &["distribution"];

const MAGIC: &[u8; 8] = b"AZVEGD\0\0";
const ROTATION_XY_SCALE: f32 = 1.0 / 255.0;
const ROTATION_Z_SCALE: f32 = 1.0 / 511.0;
const SCALE_FACTOR: f32 = 0.01;
const DYNAMIC_SLICE_SOURCE_PREFIX: &str = "slices/";
const DYNAMIC_SLICE_SOURCE_EXTENSION: &str = ".dynamicslice";

/// Region-level vegetation descriptors and placements.
#[derive(Asset, TypePath, Debug, Clone, PartialEq, Eq)]
pub struct VegetationDistributionAsset {
    pub version: u32,
    names: Box<str>,
    descriptors: Box<[VegetationDistributionDescriptor]>,
    placements: Box<[VegetationDistributionPlacement]>,
    point_layers: [Box<[VegetationDistributionPoint]>; 2],
}

impl az_core::AzTypeInfo for VegetationDistributionAsset {
    const NAME: &'static str = "Azoth::VegetationDistributionAsset";
    const TYPE_ID: uuid::Uuid = uuid::uuid!("d4f1a2c5-09e6-4d0f-86b7-7c4b2e3a4912");
}

impl az_core::AzRtti for VegetationDistributionAsset {}

impl az_core::AssetData for VegetationDistributionAsset {
    const STABLE_NAME: &'static str = "azoth.vegetation.distribution";
}

impl VegetationDistributionAsset {
    /// Create a vegetation distribution asset from validated parts.
    ///
    /// # Errors
    ///
    /// Returns an error when string ranges are invalid or a placement points
    /// outside the descriptor table.
    pub fn new(
        names: Box<str>,
        descriptors: Box<[VegetationDistributionDescriptor]>,
        placements: Box<[VegetationDistributionPlacement]>,
        point_layers: [Box<[VegetationDistributionPoint]>; 2],
    ) -> Result<Self, VegetationDistributionAssetFormatError> {
        validate_parts(&names, &descriptors, &placements)?;
        Ok(Self {
            version: VEGETATION_DISTRIBUTION_ASSET_VERSION,
            names,
            descriptors,
            placements,
            point_layers,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub fn names(&self) -> &str {
        &self.names
    }

    #[must_use]
    pub const fn descriptors(&self) -> &[VegetationDistributionDescriptor] {
        &self.descriptors
    }

    #[must_use]
    pub const fn placements(&self) -> &[VegetationDistributionPlacement] {
        &self.placements
    }

    #[must_use]
    pub const fn point_layers(&self) -> &[Box<[VegetationDistributionPoint]>; 2] {
        &self.point_layers
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.descriptors.is_empty()
            && self.placements.is_empty()
            && self.point_layers.iter().all(|layer| layer.is_empty())
    }

    #[must_use]
    pub fn is_engine_asset_path(path: &str) -> bool {
        VEGETATION_DISTRIBUTION_ASSET_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
    }
}

/// Descriptor entry referenced by vegetation placement rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VegetationDistributionDescriptor {
    pub slice_path: Range<u32>,
    pub variant: Range<u32>,
}

impl VegetationDistributionDescriptor {
    #[must_use]
    pub const fn new(slice_path: Range<u32>, variant: Range<u32>) -> Self {
        Self {
            slice_path,
            variant,
        }
    }

    #[must_use]
    pub fn slice_path<'a>(&self, names: &'a str) -> &'a str {
        range_str(names, &self.slice_path).unwrap_or("")
    }

    #[must_use]
    pub fn variant<'a>(&self, names: &'a str) -> &'a str {
        range_str(names, &self.variant).unwrap_or("")
    }

    #[must_use]
    pub fn dynamic_slice_source_path<'a>(&self, names: &'a str) -> Option<Cow<'a, str>> {
        dynamic_slice_source_path(self.slice_path(names))
    }
}

#[must_use]
pub fn dynamic_slice_source_path(path: &str) -> Option<Cow<'_, str>> {
    let path = path.trim();
    if path.is_empty() {
        return None;
    }

    if has_source_extension(path) {
        return Some(Cow::Borrowed(path));
    }

    match path.strip_prefix(DYNAMIC_SLICE_SOURCE_PREFIX) {
        Some(_) => Some(Cow::Owned(format!(
            "{path}{DYNAMIC_SLICE_SOURCE_EXTENSION}"
        ))),
        None => Some(Cow::Owned(format!(
            "{DYNAMIC_SLICE_SOURCE_PREFIX}{path}{DYNAMIC_SLICE_SOURCE_EXTENSION}"
        ))),
    }
}

fn has_source_extension(path: &str) -> bool {
    path.rsplit_once('/')
        .map_or(path, |(_, file_name)| file_name)
        .contains('.')
}

/// One vegetation placement in packed region coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VegetationDistributionPlacement {
    pub descriptor_index: u32,
    pub position: UVec2,
    pub rotation: VegetationPackedRotation,
    pub scale: VegetationPackedScale,
    pub height_mode: VegetationHeightMode,
}

impl VegetationDistributionPlacement {
    #[must_use]
    pub const fn new(
        descriptor_index: u32,
        position: UVec2,
        rotation: VegetationPackedRotation,
        scale: VegetationPackedScale,
        height_mode: VegetationHeightMode,
    ) -> Self {
        Self {
            descriptor_index,
            position,
            rotation,
            scale,
            height_mode,
        }
    }
}

/// Packed rotation used by vegetation placement rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VegetationPackedRotation(u32);

impl VegetationPackedRotation {
    #[inline]
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.0
    }

    #[must_use]
    pub fn to_quat(self) -> Quat {
        let raw = self.0;
        // The masks leave 10 and 11 bits, so the narrowing to `u16` is exact
        // and `f32::from` then widens losslessly.
        let x = f32::from(((raw >> 21) & 0x3ff) as u16).mul_add(ROTATION_XY_SCALE, -1.0);
        let y = f32::from(((raw >> 11) & 0x3ff) as u16).mul_add(ROTATION_XY_SCALE, -1.0);
        let z = f32::from((raw & 0x7ff) as u16).mul_add(ROTATION_Z_SCALE, -1.0);
        let xyz_len_squared = x.mul_add(x, y.mul_add(y, z * z));
        let mut w = (1.0 - xyz_len_squared.clamp(0.0, 1.0)).sqrt();
        // The top bit carries the sign of `w`.
        if raw & 0x8000_0000 != 0 {
            w = -w;
        }

        let quat = Quat::from_xyzw(x, y, z, w);
        let len_squared = quat.length_squared();
        if len_squared > 0.0 {
            quat * len_squared.sqrt().recip()
        } else {
            Quat::IDENTITY
        }
    }
}

/// Packed uniform scale used by vegetation placement rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VegetationPackedScale(u8);

impl VegetationPackedScale {
    #[inline]
    #[must_use]
    pub const fn new(value: u8) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn raw(self) -> u8 {
        self.0
    }

    #[inline]
    #[must_use]
    pub const fn as_f32(self) -> f32 {
        self.0 as f32 * SCALE_FACTOR
    }
}

/// Height sampling mode used when a placement is projected into a region.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VegetationHeightMode {
    #[default]
    Terrain,
    MaxTerrainAndSurface,
    Surface,
    Other(u8),
}

impl VegetationHeightMode {
    #[inline]
    #[must_use]
    pub const fn from_u8(value: u8) -> Self {
        match value {
            0 => Self::Terrain,
            1 => Self::MaxTerrainAndSurface,
            2 => Self::Surface,
            value => Self::Other(value),
        }
    }

    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Terrain => 0,
            Self::MaxTerrainAndSurface => 1,
            Self::Surface => 2,
            Self::Other(value) => value,
        }
    }
}

/// Tagged point in a vegetation distribution layer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VegetationDistributionPoint {
    pub position: UVec2,
    pub tag: u8,
}

impl VegetationDistributionPoint {
    #[inline]
    #[must_use]
    pub const fn new(position: UVec2, tag: u8) -> Self {
        Self { position, tag }
    }
}

/// Register vegetation distribution asset loading.
pub struct VegetationDistributionAssetPlugin;

impl Plugin for VegetationDistributionAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<VegetationDistributionAsset>()
            .init_asset_loader::<VegetationDistributionAssetLoader>();
    }
}

/// Write a vegetation distribution asset.
///
/// # Errors
///
/// Returns [`VegetationDistributionAssetFormatError::InvalidData`] if a
/// descriptor's name range falls outside the string table or splits a UTF-8
/// boundary, if a placement names a descriptor index the table does not hold,
/// if a name range ends before it starts, or if a placement or point coordinate
/// does not fit in `u16`;
/// [`VegetationDistributionAssetFormatError::TooManyItems`] if the name bytes,
/// descriptors, placements or either point layer exceed `u32`; and
/// [`VegetationDistributionAssetFormatError::Io`] if `writer` fails.
pub fn write_vegetation_distribution_asset(
    asset: &VegetationDistributionAsset,
    writer: impl Write,
) -> Result<(), VegetationDistributionAssetFormatError> {
    format::write_vegetation_distribution_asset(asset, writer)
}

/// Read a vegetation distribution asset.
///
/// # Errors
///
/// Returns any error [`read_vegetation_distribution_asset_from_reader`] returns
/// for a cursor over `bytes` — in practice
/// [`VegetationDistributionAssetFormatError::Io`] when `bytes` is shorter than
/// the header or a section it declares.
pub fn read_vegetation_distribution_asset(
    bytes: &[u8],
) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
    format::read_vegetation_distribution_asset(bytes)
}

/// Read a vegetation distribution asset from a stream.
///
/// # Errors
///
/// Returns [`VegetationDistributionAssetFormatError::BadMagic`] if the leading
/// eight bytes are not the asset magic,
/// [`VegetationDistributionAssetFormatError::UnsupportedVersion`] if the version
/// word is not [`VEGETATION_DISTRIBUTION_ASSET_VERSION`],
/// [`VegetationDistributionAssetFormatError::Utf8`] if the name table is not
/// UTF-8, [`VegetationDistributionAssetFormatError::InvalidData`] if a name
/// range overflows or the decoded tables fail validation, and
/// [`VegetationDistributionAssetFormatError::Io`] if `reader` ends early or
/// fails.
pub fn read_vegetation_distribution_asset_from_reader(
    reader: impl Read,
) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
    format::read_vegetation_distribution_asset_from_reader(reader)
}

/// Bevy asset loader for vegetation distribution assets.
#[derive(Default, TypePath)]
pub struct VegetationDistributionAssetLoader;

impl AssetLoader for VegetationDistributionAssetLoader {
    type Asset = VegetationDistributionAsset;
    type Settings = ();
    type Error = VegetationDistributionAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        format::read_vegetation_distribution_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        VEGETATION_DISTRIBUTION_ASSET_EXTENSIONS
    }
}

/// Vegetation distribution asset format errors.
#[derive(Debug, Error)]
pub enum VegetationDistributionAssetFormatError {
    #[error("bad vegetation distribution asset magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported vegetation distribution asset version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid vegetation distribution asset data: {0}")]
    InvalidData(&'static str),
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn validate_parts(
    names: &str,
    descriptors: &[VegetationDistributionDescriptor],
    placements: &[VegetationDistributionPlacement],
) -> Result<(), VegetationDistributionAssetFormatError> {
    for descriptor in descriptors {
        validate_range(names, &descriptor.slice_path)?;
        validate_range(names, &descriptor.variant)?;
    }

    let descriptor_count = descriptors.len();
    for placement in placements {
        if placement.descriptor_index as usize >= descriptor_count {
            return Err(VegetationDistributionAssetFormatError::InvalidData(
                "placement descriptor index outside descriptor table",
            ));
        }
    }
    Ok(())
}

const fn validate_range(
    names: &str,
    range: &Range<u32>,
) -> Result<(), VegetationDistributionAssetFormatError> {
    let start = range.start as usize;
    let end = range.end as usize;
    if start > end || end > names.len() {
        return Err(VegetationDistributionAssetFormatError::InvalidData(
            "string range outside string table",
        ));
    }
    if !names.is_char_boundary(start) || !names.is_char_boundary(end) {
        return Err(VegetationDistributionAssetFormatError::InvalidData(
            "string range splits UTF-8",
        ));
    }
    Ok(())
}

fn range_str<'a>(names: &'a str, range: &Range<u32>) -> Option<&'a str> {
    names.get(range.start as usize..range.end as usize)
}

fn checked_u32(
    count: usize,
    what: &'static str,
) -> Result<u32, VegetationDistributionAssetFormatError> {
    u32::try_from(count)
        .map_err(|_| VegetationDistributionAssetFormatError::TooManyItems { what, count })
}

fn range_len(range: &Range<u32>) -> Result<u32, VegetationDistributionAssetFormatError> {
    range
        .end
        .checked_sub(range.start)
        .ok_or(VegetationDistributionAssetFormatError::InvalidData(
            "string range end is before start",
        ))
}

mod format {
    use super::{
        AsyncReadExt, Box, Cursor, MAGIC, Read, Reader, Result, String, UVec2,
        VEGETATION_DISTRIBUTION_ASSET_VERSION, Vec, VegetationDistributionAsset,
        VegetationDistributionAssetFormatError, VegetationDistributionDescriptor,
        VegetationDistributionPlacement, VegetationDistributionPoint, VegetationHeightMode,
        VegetationPackedRotation, VegetationPackedScale, Write, checked_u32, range_len,
        validate_parts, vec,
    };

    pub(super) fn write_vegetation_distribution_asset(
        asset: &VegetationDistributionAsset,
        mut writer: impl Write,
    ) -> Result<(), VegetationDistributionAssetFormatError> {
        validate_parts(asset.names(), asset.descriptors(), asset.placements())?;
        writer.write_all(MAGIC)?;
        write_u32(&mut writer, VEGETATION_DISTRIBUTION_ASSET_VERSION)?;
        write_u32(
            &mut writer,
            checked_u32(asset.names().len(), "vegetation distribution name bytes")?,
        )?;
        write_u32(
            &mut writer,
            checked_u32(
                asset.descriptors().len(),
                "vegetation distribution descriptors",
            )?,
        )?;
        write_u32(
            &mut writer,
            checked_u32(
                asset.placements().len(),
                "vegetation distribution placements",
            )?,
        )?;
        write_u32(
            &mut writer,
            checked_u32(
                asset.point_layers[0].len(),
                "vegetation distribution point layer 0",
            )?,
        )?;
        write_u32(
            &mut writer,
            checked_u32(
                asset.point_layers[1].len(),
                "vegetation distribution point layer 1",
            )?,
        )?;
        writer.write_all(asset.names().as_bytes())?;

        for descriptor in asset.descriptors() {
            write_u32(&mut writer, descriptor.slice_path.start)?;
            write_u32(&mut writer, range_len(&descriptor.slice_path)?)?;
            write_u32(&mut writer, descriptor.variant.start)?;
            write_u32(&mut writer, range_len(&descriptor.variant)?)?;
        }
        for placement in asset.placements() {
            write_u32(&mut writer, placement.descriptor_index)?;
            write_u16(
                &mut writer,
                checked_u16(placement.position.x, "placement x")?,
            )?;
            write_u16(
                &mut writer,
                checked_u16(placement.position.y, "placement y")?,
            )?;
            write_u32(&mut writer, placement.rotation.raw())?;
            writer.write_all(&[placement.scale.raw(), placement.height_mode.as_u8()])?;
        }
        for layer in &asset.point_layers {
            for point in layer {
                write_u16(&mut writer, checked_u16(point.position.x, "point x")?)?;
                write_u16(&mut writer, checked_u16(point.position.y, "point y")?)?;
                writer.write_all(&[point.tag])?;
            }
        }
        Ok(())
    }

    pub(super) fn read_vegetation_distribution_asset(
        bytes: &[u8],
    ) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
        read_vegetation_distribution_asset_from_reader(Cursor::new(bytes))
    }

    pub(super) fn read_vegetation_distribution_asset_from_reader(
        mut reader: impl Read,
    ) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(VegetationDistributionAssetFormatError::BadMagic { found: magic });
        }
        read_after_magic(&mut reader)
    }

    pub(super) async fn read_vegetation_distribution_asset_from_bevy_reader(
        reader: &mut dyn Reader,
    ) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic).await?;
        if &magic != MAGIC {
            return Err(VegetationDistributionAssetFormatError::BadMagic { found: magic });
        }
        read_async_after_magic(reader).await
    }

    fn read_after_magic(
        reader: &mut impl Read,
    ) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
        let version = read_u32(reader)?;
        if version != VEGETATION_DISTRIBUTION_ASSET_VERSION {
            return Err(VegetationDistributionAssetFormatError::UnsupportedVersion {
                version,
                expected: VEGETATION_DISTRIBUTION_ASSET_VERSION,
            });
        }
        let name_len = read_u32(reader)? as usize;
        let descriptor_count = read_u32(reader)? as usize;
        let placement_count = read_u32(reader)? as usize;
        let layer0_count = read_u32(reader)? as usize;
        let layer1_count = read_u32(reader)? as usize;
        let names = read_names(reader, name_len)?;

        let mut descriptors = Vec::with_capacity(descriptor_count);
        for _ in 0..descriptor_count {
            let slice_start = read_u32(reader)?;
            let slice_len = read_u32(reader)?;
            let variant_start = read_u32(reader)?;
            let variant_len = read_u32(reader)?;
            descriptors.push(VegetationDistributionDescriptor::new(
                slice_start..checked_range_end(slice_start, slice_len)?,
                variant_start..checked_range_end(variant_start, variant_len)?,
            ));
        }

        let mut placements = Vec::with_capacity(placement_count);
        for _ in 0..placement_count {
            placements.push(read_placement(reader)?);
        }
        let point_layers = [
            read_points(reader, layer0_count)?,
            read_points(reader, layer1_count)?,
        ];

        VegetationDistributionAsset::new(
            names,
            descriptors.into_boxed_slice(),
            placements.into_boxed_slice(),
            point_layers,
        )
    }

    async fn read_async_after_magic(
        reader: &mut dyn Reader,
    ) -> Result<VegetationDistributionAsset, VegetationDistributionAssetFormatError> {
        let version = read_async_u32(reader).await?;
        if version != VEGETATION_DISTRIBUTION_ASSET_VERSION {
            return Err(VegetationDistributionAssetFormatError::UnsupportedVersion {
                version,
                expected: VEGETATION_DISTRIBUTION_ASSET_VERSION,
            });
        }
        let name_len = read_async_u32(reader).await? as usize;
        let descriptor_count = read_async_u32(reader).await? as usize;
        let placement_count = read_async_u32(reader).await? as usize;
        let layer0_count = read_async_u32(reader).await? as usize;
        let layer1_count = read_async_u32(reader).await? as usize;
        let names = read_async_names(reader, name_len).await?;

        let mut descriptors = Vec::with_capacity(descriptor_count);
        for _ in 0..descriptor_count {
            let slice_start = read_async_u32(reader).await?;
            let slice_len = read_async_u32(reader).await?;
            let variant_start = read_async_u32(reader).await?;
            let variant_len = read_async_u32(reader).await?;
            descriptors.push(VegetationDistributionDescriptor::new(
                slice_start..checked_range_end(slice_start, slice_len)?,
                variant_start..checked_range_end(variant_start, variant_len)?,
            ));
        }

        let mut placements = Vec::with_capacity(placement_count);
        for _ in 0..placement_count {
            placements.push(read_async_placement(reader).await?);
        }
        let point_layers = [
            read_async_points(reader, layer0_count).await?,
            read_async_points(reader, layer1_count).await?,
        ];

        VegetationDistributionAsset::new(
            names,
            descriptors.into_boxed_slice(),
            placements.into_boxed_slice(),
            point_layers,
        )
    }

    fn read_names(
        reader: &mut impl Read,
        len: usize,
    ) -> Result<Box<str>, VegetationDistributionAssetFormatError> {
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes)?;
        Ok(String::from_utf8(bytes)?.into_boxed_str())
    }

    async fn read_async_names(
        reader: &mut dyn Reader,
        len: usize,
    ) -> Result<Box<str>, VegetationDistributionAssetFormatError> {
        let mut bytes = vec![0; len];
        reader.read_exact(&mut bytes).await?;
        Ok(String::from_utf8(bytes)?.into_boxed_str())
    }

    fn read_placement(
        reader: &mut impl Read,
    ) -> Result<VegetationDistributionPlacement, VegetationDistributionAssetFormatError> {
        let descriptor_index = read_u32(reader)?;
        let position = UVec2::new(u32::from(read_u16(reader)?), u32::from(read_u16(reader)?));
        let rotation = VegetationPackedRotation::new(read_u32(reader)?);
        let scale = VegetationPackedScale::new(read_u8(reader)?);
        let height_mode = VegetationHeightMode::from_u8(read_u8(reader)?);
        Ok(VegetationDistributionPlacement::new(
            descriptor_index,
            position,
            rotation,
            scale,
            height_mode,
        ))
    }

    async fn read_async_placement(
        reader: &mut dyn Reader,
    ) -> Result<VegetationDistributionPlacement, VegetationDistributionAssetFormatError> {
        let descriptor_index = read_async_u32(reader).await?;
        let position = UVec2::new(
            u32::from(read_async_u16(reader).await?),
            u32::from(read_async_u16(reader).await?),
        );
        let rotation = VegetationPackedRotation::new(read_async_u32(reader).await?);
        let scale = VegetationPackedScale::new(read_async_u8(reader).await?);
        let height_mode = VegetationHeightMode::from_u8(read_async_u8(reader).await?);
        Ok(VegetationDistributionPlacement::new(
            descriptor_index,
            position,
            rotation,
            scale,
            height_mode,
        ))
    }

    fn read_points(
        reader: &mut impl Read,
        count: usize,
    ) -> Result<Box<[VegetationDistributionPoint]>, VegetationDistributionAssetFormatError> {
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            let position = UVec2::new(u32::from(read_u16(reader)?), u32::from(read_u16(reader)?));
            points.push(VegetationDistributionPoint::new(position, read_u8(reader)?));
        }
        Ok(points.into_boxed_slice())
    }

    async fn read_async_points(
        reader: &mut dyn Reader,
        count: usize,
    ) -> Result<Box<[VegetationDistributionPoint]>, VegetationDistributionAssetFormatError> {
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            let position = UVec2::new(
                u32::from(read_async_u16(reader).await?),
                u32::from(read_async_u16(reader).await?),
            );
            points.push(VegetationDistributionPoint::new(
                position,
                read_async_u8(reader).await?,
            ));
        }
        Ok(points.into_boxed_slice())
    }

    fn checked_range_end(
        start: u32,
        len: u32,
    ) -> Result<u32, VegetationDistributionAssetFormatError> {
        start
            .checked_add(len)
            .ok_or(VegetationDistributionAssetFormatError::InvalidData(
                "string range overflow",
            ))
    }

    fn checked_u16(
        value: u32,
        what: &'static str,
    ) -> Result<u16, VegetationDistributionAssetFormatError> {
        u16::try_from(value).map_err(|_| {
            VegetationDistributionAssetFormatError::InvalidData(match what {
                "placement x" => "placement x coordinate exceeds u16",
                "placement y" => "placement y coordinate exceeds u16",
                "point x" => "point x coordinate exceeds u16",
                "point y" => "point y coordinate exceeds u16",
                _ => "coordinate exceeds u16",
            })
        })
    }

    fn write_u16(writer: &mut impl Write, value: u16) -> Result<(), std::io::Error> {
        writer.write_all(&value.to_le_bytes())
    }

    fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
        writer.write_all(&value.to_le_bytes())
    }

    fn read_u8(reader: &mut impl Read) -> Result<u8, std::io::Error> {
        let mut bytes = [0; 1];
        reader.read_exact(&mut bytes)?;
        Ok(bytes[0])
    }

    fn read_u16(reader: &mut impl Read) -> Result<u16, std::io::Error> {
        let mut bytes = [0; 2];
        reader.read_exact(&mut bytes)?;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
        let mut bytes = [0; 4];
        reader.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    }

    async fn read_async_u8(reader: &mut dyn Reader) -> Result<u8, std::io::Error> {
        let mut bytes = [0; 1];
        reader.read_exact(&mut bytes).await?;
        Ok(bytes[0])
    }

    async fn read_async_u16(reader: &mut dyn Reader) -> Result<u16, std::io::Error> {
        let mut bytes = [0; 2];
        reader.read_exact(&mut bytes).await?;
        Ok(u16::from_le_bytes(bytes))
    }

    async fn read_async_u32(reader: &mut dyn Reader) -> Result<u32, std::io::Error> {
        let mut bytes = [0; 4];
        reader.read_exact(&mut bytes).await?;
        Ok(u32::from_le_bytes(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // `float_cmp`: A round trip has to reproduce the packed scale exactly; a tolerance would hide
    // a lossy encode.
    #[allow(clippy::float_cmp)]
    fn vegetation_distribution_asset_round_trips_binary_format() {
        let asset = VegetationDistributionAsset::new(
            "slices/treeoak".to_string().into_boxed_str(),
            vec![VegetationDistributionDescriptor::new(0..11, 11..14)].into_boxed_slice(),
            vec![VegetationDistributionPlacement::new(
                0,
                UVec2::new(10, 20),
                VegetationPackedRotation::new(0x0016_000b),
                VegetationPackedScale::new(125),
                VegetationHeightMode::MaxTerrainAndSurface,
            )]
            .into_boxed_slice(),
            [
                vec![VegetationDistributionPoint::new(UVec2::new(3, 4), 7)].into_boxed_slice(),
                Box::default(),
            ],
        )
        .unwrap();

        let mut bytes = Vec::new();
        write_vegetation_distribution_asset(&asset, &mut bytes).unwrap();
        let decoded = read_vegetation_distribution_asset(&bytes).unwrap();

        assert_eq!(decoded, asset);
        assert_eq!(
            decoded.descriptors()[0].slice_path(decoded.names()),
            "slices/tree"
        );
        assert_eq!(decoded.descriptors()[0].variant(decoded.names()), "oak");
        assert_eq!(
            decoded.descriptors()[0]
                .dynamic_slice_source_path(decoded.names())
                .as_deref(),
            Some("slices/tree.dynamicslice")
        );
        assert_eq!(decoded.placements()[0].scale.as_f32(), 1.25);
    }

    #[test]
    fn dynamic_slice_source_path_expands_distribution_paths() {
        assert_eq!(
            dynamic_slice_source_path("gatherables/master_tree").as_deref(),
            Some("slices/gatherables/master_tree.dynamicslice")
        );
        assert_eq!(
            dynamic_slice_source_path("slices/gatherables/master_tree").as_deref(),
            Some("slices/gatherables/master_tree.dynamicslice")
        );
        assert_eq!(
            dynamic_slice_source_path("slices/gatherables/master_tree.dynamicslice").as_deref(),
            Some("slices/gatherables/master_tree.dynamicslice")
        );
        assert_eq!(dynamic_slice_source_path("  ").as_deref(), None);
    }
}
