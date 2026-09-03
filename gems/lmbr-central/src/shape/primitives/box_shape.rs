use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::{bounding::Aabb3d, primitives::Cuboid};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{BOX_SHAPE_COMPONENT_TYPE_ID, BOX_SHAPE_CONFIG_TYPE_ID};

/// Box shape configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Shape/BoxShapeComponentBus.h:27`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "BoxShapeConfig", BOX_SHAPE_CONFIG_TYPE_ID, register)]
pub struct BoxShapeConfig {
    #[serde(rename = "Dimensions", default)]
    pub dimensions: Vec3,
}

impl Default for BoxShapeConfig {
    fn default() -> Self {
        Self {
            dimensions: Vec3::ONE,
        }
    }
}

impl BoxShapeConfig {
    #[must_use]
    pub fn bevy_cuboid(&self) -> Cuboid {
        Cuboid::from_size(self.dimensions.abs())
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        Aabb3d::new(Vec3::ZERO, self.dimensions.abs() * 0.5)
    }
}

/// Runtime box shape component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/BoxShapeComponent.cpp:128`.
#[derive(
    AzRtti,
    Component,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_rtti(
    name = "BoxShapeComponent",
    BOX_SHAPE_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.BoxShapeComponent", version = 1)]
pub struct BoxShapeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: BoxShapeConfig,
}

impl BoxShapeComponent {
    #[must_use]
    pub fn bevy_cuboid(&self) -> Cuboid {
        self.configuration.bevy_cuboid()
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        self.configuration.local_bounds()
    }
}
