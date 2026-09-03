use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Cry texture sampler filter value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaterialTextureFilter {
    None = -1,
    Point = 0,
    Linear = 1,
    Bilinear = 2,
    Trilinear = 3,
    Anisotropic2x = 4,
    Anisotropic4x = 5,
    Anisotropic8x = 6,
    Anisotropic16x = 7,
}

impl MaterialTextureFilter {
    #[must_use]
    pub const fn from_native_value(value: i32) -> Option<Self> {
        match value {
            -1 => Some(Self::None),
            0 => Some(Self::Point),
            1 => Some(Self::Linear),
            2 => Some(Self::Bilinear),
            3 => Some(Self::Trilinear),
            4 => Some(Self::Anisotropic2x),
            5 => Some(Self::Anisotropic4x),
            6 => Some(Self::Anisotropic8x),
            7 => Some(Self::Anisotropic16x),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> i32 {
        self as i32
    }
}
