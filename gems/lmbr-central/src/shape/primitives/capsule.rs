use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::{bounding::Aabb3d, primitives::Capsule3d};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{CAPSULE_SHAPE_COMPONENT_TYPE_ID, CAPSULE_SHAPE_CONFIG_TYPE_ID};

/// Capsule shape configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/include/LmbrCentral/Shape/CapsuleShapeComponentBus.h:26`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "CapsuleShapeConfig", CAPSULE_SHAPE_CONFIG_TYPE_ID, register)]
pub struct CapsuleShapeConfig {
    #[serde(rename = "Height", default)]
    pub height: f32,
    #[serde(rename = "Radius", default)]
    pub radius: f32,
}

impl Default for CapsuleShapeConfig {
    fn default() -> Self {
        Self {
            height: 1.0,
            radius: 0.25,
        }
    }
}

impl CapsuleShapeConfig {
    #[must_use]
    pub const fn cylinder_segment_length(&self) -> f32 {
        self.radius.mul_add(-2.0, self.height).max(0.0)
    }

    #[must_use]
    pub const fn bevy_capsule(&self) -> Capsule3d {
        Capsule3d::new(self.radius.max(0.0), self.cylinder_segment_length())
    }

    #[must_use]
    pub fn local_capsule_points(&self) -> (Vec3, Vec3) {
        let half_length = self.cylinder_segment_length() * 0.5;
        (Vec3::NEG_Z * half_length, Vec3::Z * half_length)
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        let radius = self.radius.max(0.0);
        let half_height = (self.height * 0.5).max(radius);
        Aabb3d::new(Vec3::ZERO, Vec3::new(radius, radius, half_height))
    }
}

/// Runtime capsule shape component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/CapsuleShapeComponent.cpp:139`.
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
    name = "CapsuleShapeComponent",
    CAPSULE_SHAPE_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.CapsuleShapeComponent", version = 1)]
pub struct CapsuleShapeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: CapsuleShapeConfig,
}

impl CapsuleShapeComponent {
    #[must_use]
    pub const fn bevy_capsule(&self) -> Capsule3d {
        self.configuration.bevy_capsule()
    }

    #[must_use]
    pub fn local_bounds(&self) -> Aabb3d {
        self.configuration.local_bounds()
    }
}
