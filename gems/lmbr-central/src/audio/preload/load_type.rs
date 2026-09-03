use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Audio preload activation mode.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioPreloadComponent.h:31`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum AudioPreloadLoadType {
    #[default]
    Auto = 0,
    Manual = 1,
}

impl AudioPreloadLoadType {
    #[must_use]
    pub const fn from_native_value(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Auto),
            1 => Some(Self::Manual),
            _ => None,
        }
    }

    #[must_use]
    pub const fn native_value(self) -> u32 {
        self as u32
    }
}
