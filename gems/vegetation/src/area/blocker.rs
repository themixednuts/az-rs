use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

use super::constants::{FOREGROUND_LAYER, PRIORITY_MAX};
use super::spawner::AreaConfig;

/// `VegetationBlockerComponent` type ID.
pub const VEGETATION_BLOCKER_COMPONENT_TYPE_ID: Uuid =
    uuid!("954683F7-7965-4686-BCDB-0F65767730D3");

/// `VegetationBlockerConfig` type ID.
pub const VEGETATION_BLOCKER_CONFIG_TYPE_ID: Uuid = uuid!("DA4B8865-B9F6-48DF-A789-7057B18BB883");

/// Vegetation blocker configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/BlockerComponent.h:35`.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct VegetationBlockerConfig {
    pub area: AreaConfig,
    pub inherit_behavior: bool,
    pub use_relative_uvw: bool,
}

impl Default for VegetationBlockerConfig {
    fn default() -> Self {
        Self {
            area: AreaConfig {
                layer: FOREGROUND_LAYER,
                priority: PRIORITY_MAX,
            },
            inherit_behavior: true,
            use_relative_uvw: false,
        }
    }
}

/// Vegetation blocker component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/BlockerComponent.cpp:90`.
#[derive(
    Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.VegetationBlockerComponent", version = 1)]
pub struct VegetationBlockerComponent {
    pub configuration: VegetationBlockerConfig,
}
