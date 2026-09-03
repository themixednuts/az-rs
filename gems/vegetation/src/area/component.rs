use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

use super::constants::{BACKGROUND_LAYER, FOREGROUND_LAYER, PRIORITY_MIN};
use super::spawner::AreaConfig;

/// `VegetationAreaComponent` type ID.
pub const VEGETATION_AREA_COMPONENT_TYPE_ID: Uuid = uuid!("ADCA8AB2-924D-463F-A0EA-27D254F9682A");

/// `VegetationAreaConfig` type ID.
pub const VEGETATION_AREA_CONFIG_TYPE_ID: Uuid = uuid!("1272ADB3-75A0-46FB-94CE-2C82D3741F09");

/// Vegetation area type.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/AreaComponentBase.cpp:30`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct VegetationAreaType {
    value: u8,
}

impl VegetationAreaType {
    pub const CLUSTER: Self = Self { value: 0 };
    pub const COVERAGE: Self = Self { value: 1 };

    #[must_use]
    pub const fn from_raw(value: u8) -> Self {
        Self { value }
    }

    #[must_use]
    pub const fn raw(self) -> u8 {
        self.value
    }

    #[must_use]
    pub const fn layer(self) -> u32 {
        match self.value {
            1 => BACKGROUND_LAYER,
            _ => FOREGROUND_LAYER,
        }
    }
}

impl Default for VegetationAreaType {
    fn default() -> Self {
        Self::CLUSTER
    }
}

/// Vegetation placement mode flags.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Default, Serialize, Deserialize)]
pub struct VegetationPlacementModeTypeFlag {
    bits: u8,
}

impl VegetationPlacementModeTypeFlag {
    pub const NONE: Self = Self { bits: 0 };

    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self { bits }
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }

    #[must_use]
    pub const fn contains(self, flags: Self) -> bool {
        (self.bits & flags.bits) == flags.bits
    }
}

/// Vegetation area configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/AreaComponentBase.cpp:69`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct VegetationAreaConfig {
    pub area_type: VegetationAreaType,
    pub placement_mode_type_flag: VegetationPlacementModeTypeFlag,
    pub priority: i32,
}

impl Default for VegetationAreaConfig {
    fn default() -> Self {
        Self {
            area_type: VegetationAreaType::default(),
            placement_mode_type_flag: VegetationPlacementModeTypeFlag::default(),
            priority: 1,
        }
    }
}

impl VegetationAreaConfig {
    #[must_use]
    pub fn area_config(self) -> AreaConfig {
        AreaConfig {
            layer: self.area_type.layer(),
            priority: u32::try_from(self.priority.saturating_sub(1)).unwrap_or(PRIORITY_MIN),
        }
    }
}

/// Vegetation area component.
#[derive(
    Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.VegetationAreaComponent", version = 1)]
pub struct VegetationAreaComponent {
    pub configuration: VegetationAreaConfig,
}

impl VegetationAreaComponent {
    #[must_use]
    pub fn area_config(&self) -> AreaConfig {
        self.configuration.area_config()
    }
}
