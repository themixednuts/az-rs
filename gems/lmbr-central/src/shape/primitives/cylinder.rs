use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::{bounding::Aabb3d, primitives::Cylinder};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{CYLINDER_SHAPE_COMPONENT_TYPE_ID, CYLINDER_SHAPE_CONFIG_TYPE_ID};

/// Cylinder shape configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Shape/CylinderShapeComponentBus.h:27`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "CylinderShapeConfig", CYLINDER_SHAPE_CONFIG_TYPE_ID, register)]
pub struct CylinderShapeConfig {
    #[serde(rename = "Height", default)]
    pub height: f32,
    #[serde(rename = "Radius", default)]
    pub radius: f32,
    #[serde(rename = "IgnoreEnds", default)]
    pub ignore_ends: bool,
}

impl Default for CylinderShapeConfig {
    fn default() -> Self {
        Self {
            height: 1.0,
            radius: 0.5,
            ignore_ends: false,
        }
    }
}

impl CylinderShapeConfig {
    #[must_use]
    pub const fn bevy_cylinder(&self) -> Cylinder {
        Cylinder::new(self.radius.max(0.0), self.height.max(0.0))
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        let radius = self.radius.max(0.0);
        Aabb3d::new(
            Vec3::ZERO,
            Vec3::new(radius, radius, self.height.max(0.0) * 0.5),
        )
    }
}

/// Runtime cylinder shape component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/CylinderShapeComponent.cpp:114`.
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
    name = "CylinderShapeComponent",
    CYLINDER_SHAPE_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.CylinderShapeComponent", version = 1)]
pub struct CylinderShapeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: CylinderShapeConfig,
}

impl CylinderShapeComponent {
    #[must_use]
    pub const fn bevy_cylinder(&self) -> Cylinder {
        self.configuration.bevy_cylinder()
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        self.configuration.local_bounds()
    }
}
