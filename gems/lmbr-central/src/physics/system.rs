use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::ids::PHYSICS_SYSTEM_COMPONENT_TYPE_UUID;

/// Runtime `CryPhysics` system marker component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Physics/PhysicsSystemComponent.cpp:141`.
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
    name = "PhysicsSystemComponent",
    PHYSICS_SYSTEM_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.PhysicsSystemComponent", version = 1)]
pub struct PhysicsSystemComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
}
