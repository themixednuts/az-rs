use bevy::prelude::*;

/// Lumberyard fog volume shape.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Include/LmbrCentral/Rendering/FogVolumeComponentBus.h:20`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FogVolumeType {
    None = -1,
    #[default]
    Ellipsoid = 0,
    RectangularPrism = 1,
}

impl FogVolumeType {
    #[must_use]
    pub const fn from_native_value(value: i32) -> Option<Self> {
        match value {
            -1 => Some(Self::None),
            0 => Some(Self::Ellipsoid),
            1 => Some(Self::RectangularPrism),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> i32 {
        self as i32
    }
}
