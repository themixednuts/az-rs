//! Compound shape component data.

use az_core::{ComponentConfig, EntityId as AzEntityId, component::Component as AzComponent};
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::CompoundShapeComponent` AZ component UUID.
pub const COMPOUND_SHAPE_COMPONENT_TYPE_ID: Uuid = uuid!("C0C817DE-843F-44C8-9FC1-989CDE66B662");

/// Lumberyard `LmbrCentral::CompoundShapeConfiguration` type UUID.
pub const COMPOUND_SHAPE_CONFIGURATION_TYPE_ID: Uuid =
    uuid!("4CEB4E5C-4CBD-4A84-88BA-87B23C103F3F");

/// Compound shape configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Shape/CompoundShapeComponentBus.h:26`.
#[derive(AzRtti, Debug, Clone, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[az_rtti(
    name = "CompoundShapeConfiguration",
    COMPOUND_SHAPE_CONFIGURATION_TYPE_ID,
    ComponentConfig,
    register
)]
pub struct CompoundShapeConfiguration {
    #[serde(rename = "Child Shape Entities", default)]
    pub child_shape_entities: Vec<AzEntityId>,
}

impl CompoundShapeConfiguration {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.child_shape_entities.is_empty()
    }

    #[must_use]
    pub const fn child_count(&self) -> usize {
        self.child_shape_entities.len()
    }
}

/// Runtime compound shape component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/CompoundShapeComponent.cpp:21`.
#[derive(
    AzRtti, Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "CompoundShapeComponent",
    COMPOUND_SHAPE_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.CompoundShapeComponent", version = 1)]
pub struct CompoundShapeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: CompoundShapeConfiguration,
}
