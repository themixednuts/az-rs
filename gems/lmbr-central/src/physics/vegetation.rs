//! Reflected vegetation-to-physics bridge marker.

use az_core::{component::Component as AzComponent, crc::Crc32};
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::VEGETATION_PHYSICS_COMPONENT_TYPE_UUID;

/// `AZ_CRC_CE("VegetationPhysicsService")` returned by the component
/// descriptor's `GetProvidedServices` implementation.
pub const VEGETATION_PHYSICS_SERVICE: Crc32 = Crc32::from_str_lower("VegetationPhysicsService");

/// Runtime `VegetationPhysicsComponent` version 0.
///
/// The component has no state or activation logic. It is a service marker used to
/// select the vegetation physics integration during component composition.
#[derive(
    AzRtti,
    Component,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_rtti(
    name = "VegetationPhysicsComponent",
    VEGETATION_PHYSICS_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.VegetationPhysicsComponent", version = 1)]
pub struct VegetationPhysicsComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
}
