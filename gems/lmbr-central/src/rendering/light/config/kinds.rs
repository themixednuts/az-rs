use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Lumberyard light type.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/LightComponent.h:46`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum LightType {
    #[default]
    Point,
    Area,
    Projector,
    Probe,
}

impl LightType {
    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Point),
            1 => Some(Self::Area),
            2 => Some(Self::Projector),
            3 => Some(Self::Probe),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u32 {
        match self {
            Self::Point => 0,
            Self::Area => 1,
            Self::Projector => 2,
            Self::Probe => 3,
        }
    }
}

/// Environment probe cubemap resolution.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum LightCubemapResolution {
    Res32,
    Res64,
    Res128,
    #[default]
    Res256,
    Res512,
}

impl LightCubemapResolution {
    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            32 => Some(Self::Res32),
            64 => Some(Self::Res64),
            128 => Some(Self::Res128),
            256 => Some(Self::Res256),
            512 => Some(Self::Res512),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u32 {
        self.pixels()
    }

    #[must_use]
    pub const fn pixels(self) -> u32 {
        match self {
            Self::Res32 => 32,
            Self::Res64 => 64,
            Self::Res128 => 128,
            Self::Res256 => 256,
            Self::Res512 => 512,
        }
    }
}

/// Cry voxel GI mode.
///
/// Lumberyard reference: `dev/Code/CryEngine/CryCommon/IEntityRenderState.h:388`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum VoxelGiMode {
    #[default]
    None,
    Static,
    Dynamic,
}

impl VoxelGiMode {
    #[must_use]
    pub const fn from_native_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::Static),
            2 => Some(Self::Dynamic),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Static => 1,
            Self::Dynamic => 2,
        }
    }
}
