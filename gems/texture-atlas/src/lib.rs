//! `TextureAtlas` Gem asset data and Bevy integration.
//!
//! O3DE reference: `Gems/TextureAtlas/Code/Include/TextureAtlas/TextureAtlas.h`.

use std::io::{Cursor, Read, Write};

use az_gem_contract::contribution;
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::image::TextureAtlasLayout;
use bevy::math::{URect, UVec2};
use bevy::prelude::*;
use thiserror::Error;

/// Current engine texture-atlas asset schema version.
pub const TEXTURE_ATLAS_ASSET_VERSION: u32 = 1;

/// File extensions claimed by the texture-atlas asset loader.
///
/// On-disk products keep the pak source extension (`.texatlasidx`);
/// the transformed Rust-canonical binary payload still uses our
/// `AZTEXAT` format family — only the filename matches the catalog so
/// `<asset_root>/<pak-source-path>` reads work.
pub const TEXTURE_ATLAS_ASSET_EXTENSIONS: &[&str] = &["texatlasidx"];

const MAGIC: &[u8; 8] = b"AZTEXAT\0";

/// Engine texture-atlas metadata with Bevy layout rectangles and stable names.
#[derive(Asset, TypePath, Debug, Clone, PartialEq, Eq)]
pub struct TextureAtlasAsset {
    pub version: u32,
    image_path: Box<str>,
    pub layout: TextureAtlasLayout,
    names: Box<str>,
    entries: Box<[TextureAtlasEntry]>,
}

impl TextureAtlasAsset {
    /// Create a texture-atlas asset from validated parts.
    ///
    /// # Errors
    ///
    /// Returns an error when entry name ranges do not point into the string
    /// table or when the entry count differs from the Bevy layout rect count.
    pub fn new(
        image_path: impl Into<Box<str>>,
        layout: TextureAtlasLayout,
        names: Box<str>,
        entries: Box<[TextureAtlasEntry]>,
    ) -> Result<Self, TextureAtlasAssetFormatError> {
        let image_path = image_path.into();
        validate_image_path(&image_path)?;
        validate_parts(&layout, &names, &entries)?;
        Ok(Self {
            version: TEXTURE_ATLAS_ASSET_VERSION,
            image_path,
            layout,
            names,
            entries,
        })
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub const fn entries(&self) -> &[TextureAtlasEntry] {
        &self.entries
    }

    #[must_use]
    pub fn names(&self) -> &str {
        &self.names
    }

    #[must_use]
    pub fn image_path(&self) -> &str {
        &self.image_path
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn region(&self, index: usize) -> Option<TextureAtlasRegion<'_>> {
        let entry = self.entries.get(index)?;
        let rect = *self.layout.textures.get(index)?;
        Some(TextureAtlasRegion {
            name: entry.name(self.names()),
            rect,
            index,
        })
    }

    #[must_use]
    pub const fn regions(&self) -> TextureAtlasRegions<'_> {
        TextureAtlasRegions {
            atlas: self,
            index: 0,
        }
    }

    #[must_use]
    pub fn find_region(&self, handle: &str) -> Option<TextureAtlasRegion<'_>> {
        let key = strip_extension(handle.trim());
        self.regions()
            .find(|region| region.name.eq_ignore_ascii_case(key))
    }

    #[must_use]
    pub fn is_engine_asset_path(path: &str) -> bool {
        TEXTURE_ATLAS_ASSET_EXTENSIONS
            .iter()
            .any(|extension| path.ends_with(extension))
    }
}

/// String-table range for one atlas region name.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct NameRange {
    pub start: u32,
    pub len: u32,
}

impl NameRange {
    #[inline]
    #[must_use]
    pub const fn new(start: u32, len: u32) -> Self {
        Self { start, len }
    }

    #[inline]
    #[must_use]
    pub const fn end(self) -> Option<u32> {
        self.start.checked_add(self.len)
    }
}

/// One region entry in an engine texture-atlas asset.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TextureAtlasEntry {
    pub name: NameRange,
}

impl TextureAtlasEntry {
    #[inline]
    #[must_use]
    pub const fn new(name: NameRange) -> Self {
        Self { name }
    }

    #[must_use]
    pub fn name(self, names: &str) -> &str {
        let start = self.name.start as usize;
        let end = self.name.end().unwrap_or(self.name.start) as usize;
        names.get(start..end).unwrap_or("")
    }
}

/// Borrowed atlas region returned by lookup and iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureAtlasRegion<'a> {
    pub name: &'a str,
    pub rect: URect,
    pub index: usize,
}

/// Iterator over named atlas regions.
#[derive(Debug, Clone)]
pub struct TextureAtlasRegions<'a> {
    atlas: &'a TextureAtlasAsset,
    index: usize,
}

impl<'a> Iterator for TextureAtlasRegions<'a> {
    type Item = TextureAtlasRegion<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let region = self.atlas.region(self.index)?;
        self.index += 1;
        Some(region)
    }
}

/// Register `TextureAtlas` asset loaders.
pub struct TextureAtlasAssetPlugin;

impl Plugin for TextureAtlasAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<TextureAtlasLayout>()
            .init_asset::<TextureAtlasAsset>()
            .init_asset_loader::<TextureAtlasAssetLoader>();
    }
}

/// Sealing is privacy: the generated `package_contribution` is the only way in.
///
/// `TextureAtlas` owns asset *data* and the Bevy loaders that read it, and
/// nothing a host keeps in a registry — the atlas asset type, product format,
/// source family and build rule all belong to `texture-atlas-assets` and reach
/// a host through this gem's `builders` bundle. An empty `register` is the
/// honest shape of that, and the compose-seam test holds it to empty so a
/// registration added later has to be declared here.
struct Package;

#[contribution]
impl az_gem_contract::Contribution for Package {
    fn register(&self, _ctx: &mut az_gem_contract::GemContext<'_, Self::Caps>) {}
}

/// Write an engine texture-atlas asset.
///
/// # Errors
///
/// Returns [`TextureAtlasAssetFormatError::TooManyItems`] if the layout rect,
/// entry, or name-table length exceeds `u32`, or
/// [`TextureAtlasAssetFormatError::Io`] if `writer` rejects a write.
pub fn write_texture_atlas_asset(
    asset: &TextureAtlasAsset,
    writer: impl Write,
) -> Result<(), TextureAtlasAssetFormatError> {
    format::write_texture_atlas_asset(asset, writer)
}

/// Read an engine texture-atlas asset.
///
/// # Errors
///
/// Returns [`TextureAtlasAssetFormatError::BadMagic`] or
/// [`TextureAtlasAssetFormatError::UnsupportedVersion`] if the header does not
/// match this format, [`TextureAtlasAssetFormatError::Io`] if `bytes` ends
/// mid-record, [`TextureAtlasAssetFormatError::Utf8`] if the name table is not
/// UTF-8, and [`TextureAtlasAssetFormatError::InvalidData`] if the decoded
/// parts fail validation (rect count and entry count disagree, or a name range
/// falls outside the name table).
pub fn read_texture_atlas_asset(
    bytes: &[u8],
) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
    format::read_texture_atlas_asset(bytes)
}

/// Read an engine texture-atlas asset from a stream.
///
/// # Errors
///
/// Returns any error [`read_texture_atlas_asset`] returns.
pub fn read_texture_atlas_asset_from_reader(
    reader: impl Read,
) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
    format::read_texture_atlas_asset_from_reader(reader)
}

/// Bevy asset loader for engine texture-atlas assets.
#[derive(Default, TypePath)]
pub struct TextureAtlasAssetLoader;

impl AssetLoader for TextureAtlasAssetLoader {
    type Asset = TextureAtlasAsset;
    type Settings = ();
    type Error = TextureAtlasAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        format::read_texture_atlas_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        TEXTURE_ATLAS_ASSET_EXTENSIONS
    }
}

/// Engine texture-atlas asset format errors.
#[derive(Debug, Error)]
pub enum TextureAtlasAssetFormatError {
    #[error("bad texture-atlas asset magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported texture-atlas asset version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid texture-atlas asset data: {0}")]
    InvalidData(&'static str),
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

fn validate_parts(
    layout: &TextureAtlasLayout,
    names: &str,
    entries: &[TextureAtlasEntry],
) -> Result<(), TextureAtlasAssetFormatError> {
    if layout.textures.len() != entries.len() {
        return Err(TextureAtlasAssetFormatError::InvalidData(
            "layout rect count does not match entry count",
        ));
    }
    for entry in entries {
        let Some(end) = entry.name.end() else {
            return Err(TextureAtlasAssetFormatError::InvalidData(
                "name range overflow",
            ));
        };
        if end as usize > names.len() || !names.is_char_boundary(entry.name.start as usize) {
            return Err(TextureAtlasAssetFormatError::InvalidData(
                "name range outside string table",
            ));
        }
        if !names.is_char_boundary(end as usize) {
            return Err(TextureAtlasAssetFormatError::InvalidData(
                "name range splits UTF-8",
            ));
        }
    }
    Ok(())
}

fn validate_image_path(path: &str) -> Result<(), TextureAtlasAssetFormatError> {
    if path.trim().is_empty() {
        return Err(TextureAtlasAssetFormatError::InvalidData(
            "texture-atlas image path is empty",
        ));
    }
    Ok(())
}

fn strip_extension(path: &str) -> &str {
    path.rsplit_once('.').map_or(path, |(base, _)| base)
}

fn checked_u32(count: usize, what: &'static str) -> Result<u32, TextureAtlasAssetFormatError> {
    u32::try_from(count).map_err(|_| TextureAtlasAssetFormatError::TooManyItems { what, count })
}

mod format {
    use super::{
        AsyncReadExt, Cursor, MAGIC, NameRange, Read, Reader, Result, String,
        TEXTURE_ATLAS_ASSET_VERSION, TextureAtlasAsset, TextureAtlasAssetFormatError,
        TextureAtlasEntry, TextureAtlasLayout, URect, UVec2, Vec, Write, checked_u32,
        validate_image_path, validate_parts, vec,
    };

    pub fn write_texture_atlas_asset(
        asset: &TextureAtlasAsset,
        mut writer: impl Write,
    ) -> Result<(), TextureAtlasAssetFormatError> {
        validate_image_path(asset.image_path())?;
        validate_parts(&asset.layout, asset.names(), asset.entries())?;
        writer.write_all(MAGIC)?;
        write_u32(&mut writer, TEXTURE_ATLAS_ASSET_VERSION)?;
        write_u32(&mut writer, asset.layout.size.x)?;
        write_u32(&mut writer, asset.layout.size.y)?;
        write_u32(
            &mut writer,
            checked_u32(asset.image_path().len(), "texture-atlas image path bytes")?,
        )?;
        write_u32(
            &mut writer,
            checked_u32(asset.names().len(), "texture-atlas name bytes")?,
        )?;
        write_u32(
            &mut writer,
            checked_u32(asset.entries().len(), "texture-atlas entries")?,
        )?;
        writer.write_all(asset.image_path().as_bytes())?;
        writer.write_all(asset.names().as_bytes())?;
        for (entry, rect) in asset.entries().iter().zip(&asset.layout.textures) {
            write_u32(&mut writer, entry.name.start)?;
            write_u32(&mut writer, entry.name.len)?;
            write_u32(&mut writer, rect.min.x)?;
            write_u32(&mut writer, rect.min.y)?;
            write_u32(&mut writer, rect.max.x)?;
            write_u32(&mut writer, rect.max.y)?;
        }
        Ok(())
    }

    pub fn read_texture_atlas_asset(
        bytes: &[u8],
    ) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
        read_texture_atlas_asset_from_reader(Cursor::new(bytes))
    }

    pub fn read_texture_atlas_asset_from_reader(
        mut reader: impl Read,
    ) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(TextureAtlasAssetFormatError::BadMagic { found: magic });
        }
        read_after_magic(&mut reader)
    }

    pub async fn read_texture_atlas_asset_from_bevy_reader(
        reader: &mut dyn Reader,
    ) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
        let mut magic = [0u8; 8];
        reader.read_exact(&mut magic).await?;
        if &magic != MAGIC {
            return Err(TextureAtlasAssetFormatError::BadMagic { found: magic });
        }
        read_async_after_magic(reader).await
    }

    fn read_after_magic(
        reader: &mut impl Read,
    ) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
        let version = read_u32(reader)?;
        if version != TEXTURE_ATLAS_ASSET_VERSION {
            return Err(TextureAtlasAssetFormatError::UnsupportedVersion {
                version,
                expected: TEXTURE_ATLAS_ASSET_VERSION,
            });
        }
        let size = UVec2::new(read_u32(reader)?, read_u32(reader)?);
        let image_path_len = read_u32(reader)? as usize;
        let name_len = read_u32(reader)? as usize;
        let entry_count = read_u32(reader)? as usize;
        let mut image_path_bytes = vec![0; image_path_len];
        reader.read_exact(&mut image_path_bytes)?;
        let image_path = String::from_utf8(image_path_bytes)?.into_boxed_str();
        let mut name_bytes = vec![0; name_len];
        reader.read_exact(&mut name_bytes)?;
        let names = String::from_utf8(name_bytes)?.into_boxed_str();

        let mut textures = Vec::with_capacity(entry_count);
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let start = read_u32(reader)?;
            let len = read_u32(reader)?;
            let min_x = read_u32(reader)?;
            let min_y = read_u32(reader)?;
            let max_x = read_u32(reader)?;
            let max_y = read_u32(reader)?;
            entries.push(TextureAtlasEntry::new(NameRange::new(start, len)));
            textures.push(URect {
                min: UVec2::new(min_x, min_y),
                max: UVec2::new(max_x, max_y),
            });
        }

        TextureAtlasAsset::new(
            image_path,
            TextureAtlasLayout { size, textures },
            names,
            entries.into_boxed_slice(),
        )
    }

    async fn read_async_after_magic(
        reader: &mut dyn Reader,
    ) -> Result<TextureAtlasAsset, TextureAtlasAssetFormatError> {
        let version = read_async_u32(reader).await?;
        if version != TEXTURE_ATLAS_ASSET_VERSION {
            return Err(TextureAtlasAssetFormatError::UnsupportedVersion {
                version,
                expected: TEXTURE_ATLAS_ASSET_VERSION,
            });
        }
        let size = UVec2::new(read_async_u32(reader).await?, read_async_u32(reader).await?);
        let image_path_len = read_async_u32(reader).await? as usize;
        let name_len = read_async_u32(reader).await? as usize;
        let entry_count = read_async_u32(reader).await? as usize;
        let mut image_path_bytes = vec![0; image_path_len];
        reader.read_exact(&mut image_path_bytes).await?;
        let image_path = String::from_utf8(image_path_bytes)?.into_boxed_str();
        let mut name_bytes = vec![0; name_len];
        reader.read_exact(&mut name_bytes).await?;
        let names = String::from_utf8(name_bytes)?.into_boxed_str();

        let mut textures = Vec::with_capacity(entry_count);
        let mut entries = Vec::with_capacity(entry_count);
        for _ in 0..entry_count {
            let start = read_async_u32(reader).await?;
            let len = read_async_u32(reader).await?;
            let min_x = read_async_u32(reader).await?;
            let min_y = read_async_u32(reader).await?;
            let max_x = read_async_u32(reader).await?;
            let max_y = read_async_u32(reader).await?;
            entries.push(TextureAtlasEntry::new(NameRange::new(start, len)));
            textures.push(URect {
                min: UVec2::new(min_x, min_y),
                max: UVec2::new(max_x, max_y),
            });
        }

        TextureAtlasAsset::new(
            image_path,
            TextureAtlasLayout { size, textures },
            names,
            entries.into_boxed_slice(),
        )
    }

    fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
        writer.write_all(&value.to_le_bytes())
    }

    fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
        let mut bytes = [0; 4];
        reader.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
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
    fn texture_atlas_asset_round_trips_binary_format() {
        let names = "lyshineui/images/icon".to_string().into_boxed_str();
        let name_len = u32::try_from(names.len()).expect("test name table fits in u32");
        let asset = TextureAtlasAsset::new(
            "lyshineui/images/textureatlas/common.dds",
            TextureAtlasLayout {
                size: UVec2::new(32, 16),
                textures: vec![URect {
                    min: UVec2::new(4, 2),
                    max: UVec2::new(12, 10),
                }],
            },
            names,
            vec![TextureAtlasEntry::new(NameRange::new(0, name_len))].into_boxed_slice(),
        )
        .unwrap();

        let mut bytes = Vec::new();
        write_texture_atlas_asset(&asset, &mut bytes).unwrap();
        let decoded = read_texture_atlas_asset(&bytes).unwrap();

        assert_eq!(decoded, asset);
        assert_eq!(
            decoded.image_path(),
            "lyshineui/images/textureatlas/common.dds"
        );
        assert_eq!(
            decoded
                .find_region("lyshineui/images/icon.dds")
                .unwrap()
                .rect,
            asset.layout.textures[0]
        );
    }
}
