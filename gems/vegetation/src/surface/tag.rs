use az_core::crc::Crc32;
use bevy::prelude::*;

use super::constants::{TERRAIN_HOLE_TAG_NAME, TERRAIN_TAG_NAME, UNASSIGNED_TAG_NAME};

/// A vegetation surface tag stored as an AZ CRC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Reflect)]
#[repr(transparent)]
pub struct VegetationSurfaceTag {
    pub surface_tag_crc: u32,
}

impl Default for VegetationSurfaceTag {
    fn default() -> Self {
        Self::UNASSIGNED
    }
}

impl VegetationSurfaceTag {
    pub const UNASSIGNED: Self =
        Self::from_raw_crc(Crc32::from_str_lower(UNASSIGNED_TAG_NAME).value());
    pub const TERRAIN_HOLE: Self =
        Self::from_raw_crc(Crc32::from_str_lower(TERRAIN_HOLE_TAG_NAME).value());
    pub const TERRAIN: Self = Self::from_raw_crc(Crc32::from_str_lower(TERRAIN_TAG_NAME).value());

    #[must_use]
    pub const fn from_raw_crc(surface_tag_crc: u32) -> Self {
        Self { surface_tag_crc }
    }

    #[must_use]
    pub const fn from_crc32(value: Crc32) -> Self {
        Self::from_raw_crc(value.value())
    }

    #[must_use]
    pub const fn from_name(value: &str) -> Self {
        Self::from_crc32(Crc32::from_str_lower(value))
    }

    #[must_use]
    pub const fn crc32(self) -> Crc32 {
        Crc32::from_u32(self.surface_tag_crc)
    }
}

impl From<Crc32> for VegetationSurfaceTag {
    fn from(value: Crc32) -> Self {
        Self::from_crc32(value)
    }
}

impl From<VegetationSurfaceTag> for Crc32 {
    fn from(value: VegetationSurfaceTag) -> Self {
        value.crc32()
    }
}
