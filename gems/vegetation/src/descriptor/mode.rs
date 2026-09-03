//! Vegetation descriptor mode enums.

use bevy::prelude::*;

/// Radius source for vegetation bounds.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Descriptor.h:46`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum BoundMode {
    #[default]
    Radius,
    MeshRadius,
}

/// Descriptor override mode.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Descriptor.h:52`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum OverrideMode {
    #[default]
    Disable,
    Replace,
    Extend,
}

impl OverrideMode {
    #[must_use]
    pub const fn from_native_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Disable),
            1 => Some(Self::Replace),
            2 => Some(Self::Extend),
            _ => None,
        }
    }
}

/// Descriptor source mode.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/DescriptorListRequestBus.h:20`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum VegetationDescriptorSourceType {
    #[default]
    Embedded,
    External,
}
