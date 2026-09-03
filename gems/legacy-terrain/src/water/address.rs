//! Water quadtree addressing.

use bevy::prelude::*;

use crate::heightmap::math::ExactF32;

/// Level-major address of a water quadtree node.
///
/// Lumberyard reference: `dev/Code/CryEngine/Cry3DEngine/3dEngineOctreeCompile.cpp:313`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect)]
pub struct WaterNodeAddress {
    pub level: u8,
    pub x: u32,
    pub y: u32,
}

impl WaterNodeAddress {
    #[must_use]
    pub const fn new(level: u8, x: u32, y: u32) -> Self {
        Self { level, x, y }
    }

    #[must_use]
    pub fn from_level_major_index(index: usize) -> Option<Self> {
        let mut level = 0u8;
        let mut side = 1usize;
        let mut base = 0usize;

        loop {
            let level_count = side.checked_mul(side)?;
            let next_base = base.checked_add(level_count)?;
            if index < next_base {
                let offset = index - base;
                let x = u32::try_from(offset % side).ok()?;
                let y = u32::try_from(offset / side).ok()?;
                return Some(Self { level, x, y });
            }

            base = next_base;
            side = side.checked_mul(2)?;
            level = level.checked_add(1)?;
        }
    }

    #[must_use]
    pub fn level_major_index(self) -> Option<usize> {
        let side = level_side(self.level)?;
        if self.x >= side || self.y >= side {
            return None;
        }

        let base = level_base(self.level)?;
        let row = usize::try_from(self.y).ok()?.checked_mul(side as usize)?;
        let column = usize::try_from(self.x).ok()?;
        base.checked_add(row)?.checked_add(column)
    }

    #[must_use]
    pub fn child(self, offset_x: u32, offset_y: u32) -> Option<Self> {
        if offset_x > 1 || offset_y > 1 {
            return None;
        }

        Some(Self {
            level: self.level.checked_add(1)?,
            x: self.x.checked_mul(2)?.checked_add(offset_x)?,
            y: self.y.checked_mul(2)?.checked_add(offset_y)?,
        })
    }

    #[must_use]
    pub fn tile_size(self, region_size: f32) -> Option<f32> {
        if !region_size.is_finite() || region_size <= 0.0 {
            return None;
        }

        Some(region_size / level_side(self.level)?.exact_f32())
    }

    #[must_use]
    pub fn min(self, origin: Vec2, region_size: f32) -> Option<Vec2> {
        let tile_size = self.tile_size(region_size)?;
        Some(Vec2::new(
            self.x.exact_f32().mul_add(tile_size, origin.x),
            self.y.exact_f32().mul_add(tile_size, origin.y),
        ))
    }
}

pub(super) fn level_side(level: u8) -> Option<u32> {
    1u32.checked_shl(u32::from(level))
}

pub(super) fn level_base(level: u8) -> Option<usize> {
    let mut base = 0usize;
    let mut side = 1usize;
    for _ in 0..level {
        base = base.checked_add(side.checked_mul(side)?)?;
        side = side.checked_mul(2)?;
    }
    Some(base)
}
