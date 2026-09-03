use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    MESH_COLLIDER_COMPONENT_TYPE_UUID, PRIMITIVE_COLLIDER_COMPONENT_TYPE_UUID,
    PRIMITIVE_COLLIDER_CONFIG_TYPE_UUID,
};
use crate::non_empty_path;

/// Primitive collider configuration.
///
/// Lumberyard reference: `dev/Code/Framework/AzFramework/AzFramework/Physics/LegacyColliderComponentBus.h:57`.
#[derive(AzRtti, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_rtti(
    name = "PrimitiveColliderConfig",
    PRIMITIVE_COLLIDER_CONFIG_TYPE_UUID,
    register
)]
pub struct PrimitiveColliderConfig {
    #[serde(rename = "SurfaceTypeName", default)]
    pub surface_type_name: String,
}

impl PrimitiveColliderConfig {
    #[must_use]
    pub fn surface_type_name(&self) -> Option<&str> {
        non_empty_path(Some(self.surface_type_name.as_str()))
    }
}

/// Runtime primitive collider component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Physics/Colliders/PrimitiveColliderComponent.cpp:90`.
#[derive(
    AzRtti, Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "PrimitiveColliderComponent",
    PRIMITIVE_COLLIDER_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.PrimitiveColliderComponent", version = 1)]
pub struct PrimitiveColliderComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: PrimitiveColliderConfig,
}

/// Runtime mesh collider component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Physics/Colliders/MeshColliderComponent.cpp:37`.
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
    name = "MeshColliderComponent",
    MESH_COLLIDER_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.MeshColliderComponent", version = 1)]
pub struct MeshColliderComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
}
