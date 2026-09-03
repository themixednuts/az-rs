use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Cry decal projection mode.
///
/// Lumberyard reference: `dev/Code/CryEngine/CryCommon/IEntityRenderState.h:795`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum DecalProjectionType {
    #[default]
    Planar,
    ProjectOnTerrain,
    ProjectOnTerrainAndStaticObjects,
}

impl DecalProjectionType {
    #[must_use]
    pub const fn from_native_value(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Planar),
            1 => Some(Self::ProjectOnTerrain),
            2 => Some(Self::ProjectOnTerrainAndStaticObjects),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> i32 {
        match self {
            Self::Planar => 0,
            Self::ProjectOnTerrain => 1,
            Self::ProjectOnTerrainAndStaticObjects => 2,
        }
    }
}
