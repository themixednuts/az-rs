use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::{bounding::BoundingSphere, primitives::Sphere};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{SPHERE_SHAPE_COMPONENT_TYPE_ID, SPHERE_SHAPE_CONFIG_TYPE_ID};

/// Sphere shape configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Shape/SphereShapeComponentBus.h:27`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "SphereShapeConfig", SPHERE_SHAPE_CONFIG_TYPE_ID, register)]
pub struct SphereShapeConfig {
    #[serde(rename = "Radius", default)]
    pub radius: f32,
}

impl Default for SphereShapeConfig {
    fn default() -> Self {
        Self { radius: 0.5 }
    }
}

impl SphereShapeConfig {
    #[must_use]
    pub const fn bevy_sphere(&self) -> Sphere {
        Sphere::new(self.radius.max(0.0))
    }

    #[must_use]
    pub fn local_bounds(&self) -> BoundingSphere {
        BoundingSphere::new(Vec3::ZERO, self.radius.max(0.0))
    }
}

/// Runtime sphere shape component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/SphereShapeComponent.cpp:146`.
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
    name = "SphereShapeComponent",
    SPHERE_SHAPE_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.SphereShapeComponent", version = 1)]
pub struct SphereShapeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: SphereShapeConfig,
}

impl SphereShapeComponent {
    #[must_use]
    pub const fn bevy_sphere(&self) -> Sphere {
        self.configuration.bevy_sphere()
    }

    #[must_use]
    pub fn local_bounds(&self) -> BoundingSphere {
        self.configuration.local_bounds()
    }
}
