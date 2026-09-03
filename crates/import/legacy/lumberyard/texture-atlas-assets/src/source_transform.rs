//! Legacy `TextureAtlas` source transforms.

use az_asset::{EngineTextureFormat, normalize_source_path};
use az_asset_builder::{
    LegacySourceInput, LegacySourceOutput, LegacySourceTransform, SourceSchemaType,
};
use az_gem_texture_atlas::{NameRange, TextureAtlasAsset, TextureAtlasEntry};
use bevy::image::TextureAtlasLayout;
use bevy::math::{URect, UVec2};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use texture_atlas::visit_texture_atlas_index;

use crate::{
    TextureAtlasTransformError, source_schemas, texture_atlas_image_engine_path,
    texture_region_engine_key,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureAtlasSource {
    pub source_path: String,
    pub image: String,
    pub size: [u32; 2],
    pub regions: Vec<TextureAtlasSourceRegion>,
}

impl TextureAtlasSource {
    /// Builds the authoring source from a legacy `.texatlasidx` index.
    ///
    /// # Errors
    ///
    /// Returns [`TextureAtlasTransformError::Parse`] wrapping the
    /// [`TextureAtlasParseError`] from a malformed index — non-UTF-8 bytes,
    /// malformed XML, an unsupported `ObjectStream` or `TextureAtlasImpl`
    /// version, an element or attribute the layout does not allow, or a
    /// missing, non-numeric, negative or overflowing region coordinate.
    pub fn from_legacy_index(
        source_path: &str,
        bytes: &[u8],
        texture_format: EngineTextureFormat,
    ) -> Result<Self, TextureAtlasTransformError> {
        let mut regions = Vec::new();
        let stats = visit_texture_atlas_index(bytes, |region| {
            regions.push(TextureAtlasSourceRegion::from_rect(
                texture_region_engine_key(region.name, texture_format),
                region.rect,
            ));
        })?;

        Ok(Self {
            source_path: normalize_source_path(source_path),
            image: texture_atlas_image_engine_path(source_path, texture_format),
            size: [stats.size.x, stats.size.y],
            regions,
        })
    }

    /// Packs the authoring source into the runtime atlas asset, flattening
    /// region names into one string plus a table of ranges.
    ///
    /// # Errors
    ///
    /// Returns [`TextureAtlasTransformError::TooLarge`] if the accumulated
    /// name buffer offset or a single region name's length exceeds `u32`,
    /// [`TextureAtlasTransformError::CoordinateOverflow`] if a region's
    /// left + width or top + height exceeds `u32`, and
    /// [`TextureAtlasTransformError::Format`] if
    /// [`TextureAtlasAsset::new`] rejects the assembled layout.
    pub fn to_asset(&self) -> Result<TextureAtlasAsset, TextureAtlasTransformError> {
        let mut names = String::new();
        let mut entries = Vec::with_capacity(self.regions.len());
        let mut textures = Vec::with_capacity(self.regions.len());

        for region in &self.regions {
            let start = names.len();
            names.push_str(&region.name);
            entries.push(TextureAtlasEntry::new(NameRange::new(
                checked_u32(start, "texture-atlas name offset")?,
                checked_u32(region.name.len(), "texture-atlas name length")?,
            )));
            textures.push(region.rect()?);
        }

        TextureAtlasAsset::new(
            self.image.clone().into_boxed_str(),
            TextureAtlasLayout {
                size: UVec2::new(self.size[0], self.size[1]),
                textures,
            },
            names.into_boxed_str(),
            entries.into_boxed_slice(),
        )
        .map_err(TextureAtlasTransformError::Format)
    }

    /// Serialises this source to pretty-printed RON bytes with a trailing
    /// newline.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] the serializer reports. The fields here are
    /// strings and integers that RON always accepts, so this is reachable only
    /// through a serializer-internal failure.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::default())?;
        Ok(format!("{ron}\n").into_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureAtlasSourceRegion {
    pub name: String,
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

impl TextureAtlasSourceRegion {
    #[must_use]
    pub const fn from_rect(name: String, rect: URect) -> Self {
        Self {
            name,
            left: rect.min.x,
            top: rect.min.y,
            width: rect.max.x.saturating_sub(rect.min.x),
            height: rect.max.y.saturating_sub(rect.min.y),
        }
    }

    fn rect(&self) -> Result<URect, TextureAtlasTransformError> {
        let right = self.left.checked_add(self.width).ok_or(
            TextureAtlasTransformError::CoordinateOverflow {
                field: "Width",
                base: self.left,
                size: self.width,
            },
        )?;
        let bottom = self.top.checked_add(self.height).ok_or(
            TextureAtlasTransformError::CoordinateOverflow {
                field: "Height",
                base: self.top,
                size: self.height,
            },
        )?;
        Ok(URect {
            min: UVec2::new(self.left, self.top),
            max: UVec2::new(right, bottom),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureAtlasSourceTransform {
    texture_format: EngineTextureFormat,
}

impl TextureAtlasSourceTransform {
    #[must_use]
    pub const fn new(texture_format: EngineTextureFormat) -> Self {
        Self { texture_format }
    }
}

impl LegacySourceTransform for TextureAtlasSourceTransform {
    type Error = TextureAtlasSourceTransformError;

    fn transform(&self, input: LegacySourceInput<'_>) -> Result<LegacySourceOutput, Self::Error> {
        let source = TextureAtlasSource::from_legacy_index(
            &input.source_path,
            input.bytes,
            self.texture_format,
        )?;
        Ok(LegacySourceOutput::authoring_source(
            texture_atlas_source_path(&input.source_path),
            texture_atlas_schema(),
            source.to_ron_bytes()?,
        ))
    }
}

#[must_use]
pub const fn texture_atlas_schema() -> SourceSchemaType {
    source_schemas::TEXTURE_ATLAS
}

#[must_use]
pub fn texture_atlas_source_path(source_path: &str) -> String {
    let normalized = normalize_source_path(source_path);
    let stem = normalized
        .strip_suffix(".texatlasidx")
        .unwrap_or(&normalized);
    if let Some(rest) = stem.strip_prefix("lyshineui/") {
        return format!("ui/lyshineui/{rest}.atlas.ron");
    }
    format!("textures/atlas/{stem}.atlas.ron")
}

fn checked_u32(value: usize, what: &'static str) -> Result<u32, TextureAtlasTransformError> {
    u32::try_from(value).map_err(|_| TextureAtlasTransformError::TooLarge { what, value })
}

#[derive(Debug, thiserror::Error)]
pub enum TextureAtlasSourceTransformError {
    #[error("transform texture atlas source: {0}")]
    Transform(#[from] TextureAtlasTransformError),
    #[error("serialize texture atlas source RON: {0}")]
    Serialize(#[from] ron::Error),
}
