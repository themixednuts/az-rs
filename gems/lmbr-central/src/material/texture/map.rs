use std::borrow::Cow;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Texture slots used by material assets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaterialTextureMap {
    Bumpmap,
    Custom,
    Decal,
    Detail,
    Diffuse,
    Emittance,
    Environment,
    Heightmap,
    Occlusion,
    Opacity,
    SecondSmoothness,
    Smoothness,
    Specular,
    Specular2,
    SubSurface,
    NumberedCustom(u8),
    NumberedSmoothness(u8),
    Unknown(String),
}

impl MaterialTextureMap {
    #[must_use]
    pub fn from_native_name(value: &str) -> Self {
        match value.trim() {
            "Bumpmap" => Self::Bumpmap,
            "Custom" => Self::Custom,
            "Decal" => Self::Decal,
            "Detail" => Self::Detail,
            "Diffuse" => Self::Diffuse,
            "Emittance" => Self::Emittance,
            "Environment" => Self::Environment,
            "Heightmap" => Self::Heightmap,
            "Occlusion" => Self::Occlusion,
            "Opacity" => Self::Opacity,
            "SecondSmoothness" => Self::SecondSmoothness,
            "Smoothness" => Self::Smoothness,
            "Specular" => Self::Specular,
            "Specular2" => Self::Specular2,
            "SubSurface" => Self::SubSurface,
            "[1] Custom" => Self::NumberedCustom(1),
            "[2] Custom" => Self::NumberedCustom(2),
            "[3] Custom" => Self::NumberedCustom(3),
            "[4] Custom" => Self::NumberedCustom(4),
            "[5] Custom" => Self::NumberedCustom(5),
            "[5] Smoothness" => Self::NumberedSmoothness(5),
            other => Self::Unknown(other.to_string()),
        }
    }

    #[must_use]
    pub fn native_name(&self) -> Cow<'_, str> {
        match self {
            Self::Bumpmap => Cow::Borrowed("Bumpmap"),
            Self::Custom => Cow::Borrowed("Custom"),
            Self::Decal => Cow::Borrowed("Decal"),
            Self::Detail => Cow::Borrowed("Detail"),
            Self::Diffuse => Cow::Borrowed("Diffuse"),
            Self::Emittance => Cow::Borrowed("Emittance"),
            Self::Environment => Cow::Borrowed("Environment"),
            Self::Heightmap => Cow::Borrowed("Heightmap"),
            Self::Occlusion => Cow::Borrowed("Occlusion"),
            Self::Opacity => Cow::Borrowed("Opacity"),
            Self::SecondSmoothness => Cow::Borrowed("SecondSmoothness"),
            Self::Smoothness => Cow::Borrowed("Smoothness"),
            Self::Specular => Cow::Borrowed("Specular"),
            Self::Specular2 => Cow::Borrowed("Specular2"),
            Self::SubSurface => Cow::Borrowed("SubSurface"),
            Self::NumberedCustom(index) => Cow::Owned(format!("[{index}] Custom")),
            Self::NumberedSmoothness(index) => Cow::Owned(format!("[{index}] Smoothness")),
            Self::Unknown(value) => Cow::Borrowed(value.as_str()),
        }
    }
}
