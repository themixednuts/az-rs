//! AZ math types that do not have a direct glam / Bevy equivalent.

use bevy_reflect::Reflect;
use glam::Vec2;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// `AZ::RandomDistributionType`.
///
/// Source uses this when choosing between native random distributions for
/// shape point generation and timed spawning.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Reflect)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum RandomDistributionType {
    Normal,
    #[default]
    UniformReal,
    Unknown(u32),
}

impl From<u32> for RandomDistributionType {
    #[inline]
    fn from(value: u32) -> Self {
        match value {
            0 => Self::Normal,
            1 => Self::UniformReal,
            value => Self::Unknown(value),
        }
    }
}

impl From<RandomDistributionType> for u32 {
    #[inline]
    fn from(value: RandomDistributionType) -> Self {
        match value {
            RandomDistributionType::Normal => 0,
            RandomDistributionType::UniformReal => 1,
            RandomDistributionType::Unknown(value) => value,
        }
    }
}

/// `AZ::Bounds`: 2D min/max bounds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    pub min: Vec2,
    pub max: Vec2,
}

impl Bounds {
    #[must_use]
    pub const fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }
}
