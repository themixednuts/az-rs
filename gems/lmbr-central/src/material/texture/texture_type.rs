use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Cry texture dimension/source type value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MaterialTextureType {
    OneDimensional = 0,
    #[default]
    TwoDimensional = 1,
    ThreeDimensional = 2,
    Cube = 3,
    CubeArray = 4,
    DynamicTwoDimensional = 5,
    User = 6,
    NearestCube = 7,
}

impl MaterialTextureType {
    #[must_use]
    pub const fn from_native_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::OneDimensional),
            1 => Some(Self::TwoDimensional),
            2 => Some(Self::ThreeDimensional),
            3 => Some(Self::Cube),
            4 => Some(Self::CubeArray),
            5 => Some(Self::DynamicTwoDimensional),
            6 => Some(Self::User),
            7 => Some(Self::NearestCube),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> i32 {
        self as i32
    }
}
