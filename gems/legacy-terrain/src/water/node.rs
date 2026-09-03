//! Water quadtree node data.

use bevy::prelude::*;

/// Height used by empty water nodes.
pub const WATER_NODE_EMPTY_HEIGHT: f32 = 65_535.0;

/// One serialized node in a water quadtree.
///
/// Source: `resources/serialize.json`, type
/// `79BCCE0C-D451-47C0-B2A1-5CAD1D7313BD`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect)]
pub struct WaterNodeData {
    pub height: f32,
    pub floor_height: f32,
    pub flags: WaterNodeFlags,
}

impl WaterNodeData {
    #[must_use]
    pub fn has_surface(self) -> bool {
        self.height.is_finite() && self.height < WATER_NODE_EMPTY_HEIGHT && !self.flags.is_empty()
    }
}

/// Raw water-node flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub struct WaterNodeFlags {
    pub bits: u32,
}

impl WaterNodeFlags {
    #[must_use]
    pub const fn from_bits(bits: u32) -> Self {
        Self { bits }
    }

    #[must_use]
    pub const fn bits(self) -> u32 {
        self.bits
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    #[must_use]
    pub const fn contains(self, mask: u32) -> bool {
        self.bits & mask == mask
    }
}
