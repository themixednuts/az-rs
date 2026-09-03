//! Native Azoth texture settings authored-source schemas.
//!
//! This crate owns the editor/project-facing schema for `.texture.ron`
//! sidecars. It intentionally stays free of legacy texture decoders and
//! builder dependencies. Editable texture pixels live in the paired PNG/EXR
//! source image, including alpha; `.texture.ron` records build settings and
//! channel semantics only.

use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};

pub const TEXTURE_SETTINGS_SOURCE_SCHEMA_NAME: &str = "azoth.texture.TextureSettings";

/// Authoring image format selected by the offline extractor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureAuthoringFormat {
    Png8,
    Png16,
    Exr,
}

impl TextureAuthoringFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png8 | Self::Png16 => "png",
            Self::Exr => "exr",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureColorSpace {
    Srgb,
    Linear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureRole {
    Albedo,
    Normal,
    Orm,
    /// Linear-purpose channel data such as opacity, height, roughness, or
    /// other masks. The generic mask role preserves every authored channel.
    Mask,
    Data,
    Ui,
    Hdr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureNormalConvention {
    DirectX,
    OpenGl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureNormalSemantics {
    pub convention: TextureNormalConvention,
    pub reconstruct_z: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureChannel {
    R,
    G,
    B,
    A,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureOrmSemantics {
    pub occlusion: TextureChannel,
    pub roughness: TextureChannel,
    pub metallic: TextureChannel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureMipSettings {
    pub generate: bool,
    pub preserve_source_mips: bool,
    pub persistent_mip_count: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureCompressionFormat {
    Bc5,
    Bc6h,
    Bc7Srgb,
    Bc7Unorm,
    Uncompressed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureCompressionIntent {
    pub format: TextureCompressionFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureDimension {
    One,
    Two,
    Three,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureImageOrder {
    LayerFaceDepth,
}

/// Shape of an authored texture whose individual images are stacked vertically
/// in one editable PNG or EXR.
///
/// Images use KTX2 ordering: array layer, then cubemap face, then volume slice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureSourceShape {
    pub dimension: TextureDimension,
    pub image_height: u32,
    pub depth: u32,
    pub array_layers: u32,
    pub faces: u32,
    pub image_order: TextureImageOrder,
}

impl TextureSourceShape {
    #[must_use]
    pub fn image_count(&self) -> u64 {
        u64::from(self.depth) * u64::from(self.array_layers) * u64::from(self.faces)
    }

    #[must_use]
    pub fn atlas_height(&self) -> Option<u64> {
        u64::from(self.image_height).checked_mul(self.image_count())
    }
}

/// Editable texture settings RON source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextureSourceSettings {
    pub authoring_format: TextureAuthoringFormat,
    pub color_space: TextureColorSpace,
    pub role: TextureRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal_semantics: Option<TextureNormalSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orm_semantics: Option<TextureOrmSemantics>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mips: Option<TextureMipSettings>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<TextureCompressionIntent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<TextureSourceShape>,
}

impl TextureSourceSettings {
    /// Serializes these settings to pretty-printed RON bytes.
    ///
    /// # Errors
    ///
    /// Returns the [`ron::Error`] raised by `ron::ser::to_string_pretty` if the
    /// settings cannot be serialized.
    pub fn to_ron_bytes(&self) -> Result<Vec<u8>, ron::Error> {
        let ron = ron::ser::to_string_pretty(self, PrettyConfig::new())?;
        Ok(ron.into_bytes())
    }

    /// Parses settings from RON bytes.
    ///
    /// # Errors
    ///
    /// Returns a [`ron::error::SpannedError`] if `bytes` is not valid RON, or if
    /// it does not describe a [`TextureSourceSettings`] — a missing required
    /// field such as `authoring_format`, `color_space`, or `role`, or an
    /// unrecognized enum variant.
    pub fn from_ron_bytes(bytes: &[u8]) -> Result<Self, ron::error::SpannedError> {
        ron::de::from_bytes(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_settings_ron_round_trips() {
        let settings = TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Srgb,
            role: TextureRole::Albedo,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: None,
            shape: None,
        };

        let bytes = settings.to_ron_bytes().unwrap();
        assert_eq!(
            TextureSourceSettings::from_ron_bytes(&bytes).unwrap(),
            settings
        );
    }

    #[test]
    fn texture_settings_optional_knobs_round_trip_when_authored() {
        let settings = TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Linear,
            role: TextureRole::Normal,
            normal_semantics: Some(TextureNormalSemantics {
                convention: TextureNormalConvention::OpenGl,
                reconstruct_z: true,
            }),
            orm_semantics: Some(TextureOrmSemantics {
                occlusion: TextureChannel::R,
                roughness: TextureChannel::G,
                metallic: TextureChannel::B,
            }),
            mips: Some(TextureMipSettings {
                generate: true,
                preserve_source_mips: false,
                persistent_mip_count: Some(4),
            }),
            compression: Some(TextureCompressionIntent {
                format: TextureCompressionFormat::Bc5,
            }),
            shape: Some(TextureSourceShape {
                dimension: TextureDimension::Two,
                image_height: 256,
                depth: 1,
                array_layers: 2,
                faces: 6,
                image_order: TextureImageOrder::LayerFaceDepth,
            }),
        };

        let bytes = settings.to_ron_bytes().unwrap();
        assert_eq!(
            TextureSourceSettings::from_ron_bytes(&bytes).unwrap(),
            settings
        );
    }

    #[test]
    fn mask_role_round_trips_with_its_stable_ron_name() {
        let settings = TextureSourceSettings {
            authoring_format: TextureAuthoringFormat::Png8,
            color_space: TextureColorSpace::Linear,
            role: TextureRole::Mask,
            normal_semantics: None,
            orm_semantics: None,
            mips: None,
            compression: None,
            shape: None,
        };

        let bytes = settings.to_ron_bytes().unwrap();
        assert!(std::str::from_utf8(&bytes).unwrap().contains("role: mask"));
        assert_eq!(
            TextureSourceSettings::from_ron_bytes(&bytes).unwrap(),
            settings
        );
    }
}
