//! Vegetation filter and modifier execution stages.

use bevy::prelude::*;

/// Filter execution stage.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/FilterRequestBus.h:23`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum FilterStage {
    #[default]
    Default,
    PreProcess,
    PostProcess,
}

impl FilterStage {
    #[must_use]
    pub const fn from_native_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Default),
            1 => Some(Self::PreProcess),
            2 => Some(Self::PostProcess),
            _ => None,
        }
    }
}

/// Modifier execution stage.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/ModifierRequestBus.h:21`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum ModifierStage {
    PreProcess,
    #[default]
    Standard,
    PostProcess,
}
