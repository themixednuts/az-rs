use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    PhysicsComponent, STATIC_PHYSICS_COMPONENT_TYPE_UUID, STATIC_PHYSICS_CONFIG_TYPE_UUID,
};
use crate::non_empty_path;

/// Static physics configuration.
///
/// Lumberyard reference: `dev/Code/Framework/AzFramework/AzFramework/Physics/PhysicsComponentBus.h:402`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_rtti(
    name = "StaticPhysicsConfig",
    STATIC_PHYSICS_CONFIG_TYPE_UUID,
    register
)]
pub struct StaticPhysicsConfig {
    #[serde(rename = "EnabledInitially", default)]
    pub enabled_initially: bool,
    #[serde(rename = "InteractsWithTriggers", default)]
    pub interacts_with_triggers: bool,
}

impl Default for StaticPhysicsConfig {
    fn default() -> Self {
        Self {
            enabled_initially: true,
            interacts_with_triggers: false,
        }
    }
}

/// Runtime static physics component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Physics/StaticPhysicsComponent.cpp:50`.
#[derive(
    AzRtti, Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "StaticPhysicsComponent",
    STATIC_PHYSICS_COMPONENT_TYPE_UUID,
    PhysicsComponent,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.StaticPhysicsComponent", version = 1)]
pub struct StaticPhysicsComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub physics_component: PhysicsComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: StaticPhysicsConfig,
    #[serde(rename = "CollisionFilter", default)]
    pub collision_filter: String,
}

impl Default for StaticPhysicsComponent {
    fn default() -> Self {
        // The native constructor installs this collision filter.
        Self {
            physics_component: PhysicsComponent::default(),
            configuration: StaticPhysicsConfig::default(),
            collision_filter: "Structure".to_owned(),
        }
    }
}

impl StaticPhysicsComponent {
    #[inline]
    #[must_use]
    pub const fn enabled_initially(&self) -> bool {
        self.configuration.enabled_initially
    }

    #[must_use]
    pub fn collision_filter(&self) -> Option<&str> {
        non_empty_path(Some(self.collision_filter.as_str()))
    }
}
