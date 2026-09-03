//! Material asset serialization and loading.

use std::io::{Read, Write};

use az_asset::normalize_source_path;
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use thiserror::Error;

use super::constants::{
    MATERIAL_ENGINE_ASSET_EXTENSIONS, MATERIAL_OVERRIDE_ENGINE_ASSET_EXTENSIONS,
};
use super::definition::{MaterialAsset, MaterialOverrideAsset};

/// Write an engine material asset.
///
/// # Errors
///
/// [`MaterialAssetFormatError::Io`] if `writer` rejects a write, or
/// [`MaterialAssetFormatError::TooManyItems`] if the asset holds more
/// sub-materials, textures or parameters than the `u32` count the format
/// encodes.
pub fn write_material_asset(
    asset: &MaterialAsset,
    writer: impl Write,
) -> Result<(), MaterialAssetFormatError> {
    super::binary::write_material_asset(asset, writer)
}

/// Read an engine material asset.
///
/// # Errors
///
/// [`MaterialAssetFormatError::BadMagic`] if `bytes` does not open with the
/// material magic, [`MaterialAssetFormatError::UnsupportedVersion`] for a
/// version this build cannot decode, [`MaterialAssetFormatError::InvalidData`]
/// for an unknown texture filter, texture type or boolean encoding,
/// [`MaterialAssetFormatError::Utf8`] for a non-UTF-8 string, or
/// [`MaterialAssetFormatError::Io`] if `bytes` ends mid-record.
pub fn read_material_asset(bytes: &[u8]) -> Result<MaterialAsset, MaterialAssetFormatError> {
    super::binary::read_material_asset(bytes)
}

/// Read an engine material asset from a stream.
///
/// # Errors
///
/// Returns the same errors as [`read_material_asset`], with
/// [`MaterialAssetFormatError::Io`] additionally covering a `reader` failure.
pub fn read_material_asset_from_reader(
    reader: impl Read,
) -> Result<MaterialAsset, MaterialAssetFormatError> {
    super::binary::read_material_asset_from_reader(reader)
}

/// Write an engine material override asset.
///
/// # Errors
///
/// [`MaterialAssetFormatError::Io`] if `writer` rejects a write, or
/// [`MaterialAssetFormatError::TooManyItems`] if the asset holds more override
/// targets, switches or parameters than the `u32` count the format encodes.
pub fn write_material_override_asset(
    asset: &MaterialOverrideAsset,
    writer: impl Write,
) -> Result<(), MaterialAssetFormatError> {
    super::binary::write_material_override_asset(asset, writer)
}

/// Read an engine material override asset.
///
/// # Errors
///
/// [`MaterialAssetFormatError::BadMagic`] if `bytes` does not open with the
/// override magic, [`MaterialAssetFormatError::UnsupportedVersion`] for a
/// version this build cannot decode, [`MaterialAssetFormatError::InvalidData`]
/// for an unknown boolean encoding, [`MaterialAssetFormatError::Utf8`] for a
/// non-UTF-8 string, or [`MaterialAssetFormatError::Io`] if `bytes` ends
/// mid-record.
pub fn read_material_override_asset(
    bytes: &[u8],
) -> Result<MaterialOverrideAsset, MaterialAssetFormatError> {
    super::binary::read_material_override_asset(bytes)
}

/// Read an engine material override asset from a stream.
///
/// # Errors
///
/// Returns the same errors as [`read_material_override_asset`], with
/// [`MaterialAssetFormatError::Io`] additionally covering a `reader` failure.
pub fn read_material_override_asset_from_reader(
    reader: impl Read,
) -> Result<MaterialOverrideAsset, MaterialAssetFormatError> {
    super::binary::read_material_override_asset_from_reader(reader)
}

/// Resolve a `.mtl` source-path reference to the on-disk product path.
///
/// Identity-on-source-path: the extractor writes material products at
/// their pak source paths so this collapses to "normalize + ensure
/// `.mtl` suffix". Kept as a named function so call sites read
/// clearly (and so a future catalog-driven resolver has one entry
/// point to replace).
#[must_use]
pub fn material_engine_asset_path(source_path: &str) -> String {
    let mut path = normalize_source_path(source_path);
    // `normalize_source_path` has already folded ASCII case, so the
    // case-insensitive compare below is only belt and braces.
    let has_extension = std::path::Path::new(&path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mtl"));
    if !has_extension {
        path.push_str(".mtl");
    }
    path
}

/// Resolve a material-override XML source-path reference to its
/// on-disk product path.
///
/// Identity-on-source-path; see [`material_engine_asset_path`].
#[must_use]
pub fn material_override_engine_asset_path(source_path: &str) -> String {
    normalize_source_path(source_path)
}

/// Bevy asset loader for transformed material assets.
#[derive(Default, TypePath)]
pub struct MaterialAssetLoader;

impl AssetLoader for MaterialAssetLoader {
    type Asset = MaterialAsset;
    type Settings = ();
    type Error = MaterialAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        super::binary::read_material_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        MATERIAL_ENGINE_ASSET_EXTENSIONS
    }
}

/// Bevy asset loader for transformed material override assets.
#[derive(Default, TypePath)]
pub struct MaterialOverrideAssetLoader;

impl AssetLoader for MaterialOverrideAssetLoader {
    type Asset = MaterialOverrideAsset;
    type Settings = ();
    type Error = MaterialAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        super::binary::read_material_override_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        MATERIAL_OVERRIDE_ENGINE_ASSET_EXTENSIONS
    }
}

/// Error for engine material reads and writes.
#[derive(Debug, Error)]
pub enum MaterialAssetFormatError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad material asset magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported material asset version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("{what} count {count} exceeds u32")]
    TooManyItems { what: &'static str, count: usize },
    #[error("invalid material asset data: {0}")]
    InvalidData(&'static str),
    #[error("invalid UTF-8 string: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}
