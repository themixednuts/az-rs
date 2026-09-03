use az_prefab::{Prefab, ReflectPrefab};
use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

use super::spawner::AreaConfig;

/// `VegetationAreaBlenderComponent` AZ type UUID.
pub const VEGETATION_AREA_BLENDER_COMPONENT_TYPE_ID: Uuid =
    uuid!("7C051A94-079C-45A5-A94D-2005D2440A1E");

/// `VegetationAreaBlenderConfig` AZ type UUID.
pub const VEGETATION_AREA_BLENDER_CONFIG_TYPE_ID: Uuid =
    uuid!("1AC67B48-C6B1-412B-9290-662A01CD384B");

/// Vegetation area blender configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/AreaBlenderComponent.h:32`.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize, MapEntities)]
#[reflect(Component, Serialize, Deserialize)]
pub struct AreaBlenderConfig {
    pub area: AreaConfig,
    pub inherit_behavior: bool,
    pub propagate_behavior: bool,
    #[entities]
    pub operations: Vec<Entity>,
}

impl Default for AreaBlenderConfig {
    fn default() -> Self {
        Self {
            area: AreaConfig::default(),
            inherit_behavior: true,
            propagate_behavior: true,
            operations: Vec::new(),
        }
    }
}

/// Vegetation area blender component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/AreaBlenderComponent.cpp:25`.
#[derive(
    Component,
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Reflect,
    Serialize,
    Deserialize,
    MapEntities,
    Prefab,
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.AreaBlenderComponent", version = 1)]
pub struct AreaBlenderComponent {
    #[entities]
    pub configuration: AreaBlenderConfig,
}
