use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// `VegetationReferenceShapeComponent` AZ type UUID.
pub const VEGETATION_REFERENCE_SHAPE_COMPONENT_TYPE_ID: Uuid =
    uuid!("AE52A11C-3356-4178-89A2-7152E9FC869E");

/// `VegetationReferenceShapeConfig` AZ type UUID.
pub const VEGETATION_REFERENCE_SHAPE_CONFIG_TYPE_ID: Uuid =
    uuid!("BDBD474B-23E0-4722-AAF1-8CCCC98D8C5C");

/// References another entity that owns the concrete shape data.
///
/// Lumberyard reference: `dev/Gems/Vegetation/Code/Source/Components/ReferenceShapeComponent.h:29`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct ReferenceShapeConfig {
    pub shape_entity: Option<Entity>,
}

/// Shape forwarding component used by vegetation areas.
///
/// Lumberyard reference: `dev/Gems/Vegetation/Code/Source/Components/ReferenceShapeComponent.h:45`.
#[derive(
    Component, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.ReferenceShapeComponent", version = 1)]
pub struct ReferenceShapeComponent {
    pub configuration: ReferenceShapeConfig,
}
