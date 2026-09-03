use az_core::component::Component as AzComponent;
use az_derive::{AzRtti, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{
    PHYSICS_COMPONENT_TYPE_UUID, RIGID_PHYSICS_COMPONENT_TYPE_UUID, RIGID_PHYSICS_CONFIG_TYPE_UUID,
    RIGID_PHYSICS_MASS_OR_DENSITY_TYPE_UUID,
};

/// Rigid physics mass source.
///
/// Lumberyard reference: `dev/Code/Framework/AzFramework/AzFramework/Physics/PhysicsComponentBus.h:324`.
#[repr(u32)]
#[derive(AzTypeInfo, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize)]
#[az_type_info(
    name = "AzFramework::RigidPhysicsConfig::MassOrDensity",
    RIGID_PHYSICS_MASS_OR_DENSITY_TYPE_UUID
)]
#[reflect(Serialize, Deserialize)]
pub enum MassOrDensity {
    Mass = 0,
    #[default]
    Density = 1,
}

impl<'de> Deserialize<'de> for MassOrDensity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // ObjectStream dumps SpecifyMassOrDensity as a bare u32; prefab RON/json
        // may still use the unit-variant name.
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = MassOrDensity;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("MassOrDensity unit variant or integer discriminant")
            }

            fn visit_u64<E: serde::de::Error>(self, value: u64) -> Result<Self::Value, E> {
                match value {
                    0 => Ok(MassOrDensity::Mass),
                    1 => Ok(MassOrDensity::Density),
                    other => Err(E::custom(format!(
                        "unknown MassOrDensity discriminant {other}"
                    ))),
                }
            }

            fn visit_i64<E: serde::de::Error>(self, value: i64) -> Result<Self::Value, E> {
                let Ok(unsigned) = u64::try_from(value) else {
                    return Err(E::custom(format!(
                        "unknown MassOrDensity discriminant {value}"
                    )));
                };
                self.visit_u64(unsigned)
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                match value {
                    "Mass" => Ok(MassOrDensity::Mass),
                    "Density" => Ok(MassOrDensity::Density),
                    other => Err(E::unknown_variant(other, &["Mass", "Density"])),
                }
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Rigid physics configuration.
///
/// Lumberyard reference: `dev/Code/Framework/AzFramework/AzFramework/Physics/PhysicsComponentBus.h:314`.
#[derive(AzRtti, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_rtti(name = "RigidPhysicsConfig", RIGID_PHYSICS_CONFIG_TYPE_UUID, register)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the serde(rename) names ARE the ObjectStream wire field names of \
              AzFramework::RigidPhysicsConfig (PhysicsComponentBus.h:314) - \
              EnabledInitially, AtRestInitially, EnableCollisionResponse, \
              InteractsWithTriggers, RecordCollisions; packing them stops decoding"
)]
pub struct RigidPhysicsConfig {
    #[serde(rename = "EnabledInitially", default)]
    pub enabled_initially: bool,
    #[serde(rename = "SpecifyMassOrDensity", default)]
    pub specify_mass_or_density: MassOrDensity,
    #[serde(rename = "Mass", default)]
    pub mass: f32,
    #[serde(rename = "Density", default)]
    pub density: f32,
    #[serde(rename = "AtRestInitially", default)]
    pub at_rest_initially: bool,
    #[serde(rename = "EnableCollisionResponse", default)]
    pub enable_collision_response: bool,
    #[serde(rename = "InteractsWithTriggers", default)]
    pub interacts_with_triggers: bool,
    #[serde(rename = "RecordCollisions", default)]
    pub record_collisions: bool,
    #[serde(rename = "MaxRecordedCollisions", default)]
    pub max_recorded_collisions: i32,
    #[serde(rename = "SimulationDamping", default)]
    pub simulation_damping: f32,
    #[serde(rename = "SimulationMinEnergy", default)]
    pub simulation_min_energy: f32,
    #[serde(rename = "BuoyancyDamping", default)]
    pub buoyancy_damping: f32,
    #[serde(rename = "BuoyancyDensity", default)]
    pub buoyancy_density: f32,
    #[serde(rename = "BuoyancyResistance", default)]
    pub buoyancy_resistance: f32,
}

impl Default for RigidPhysicsConfig {
    fn default() -> Self {
        Self {
            enabled_initially: true,
            specify_mass_or_density: MassOrDensity::Density,
            mass: 10.0,
            density: 500.0,
            at_rest_initially: false,
            enable_collision_response: true,
            interacts_with_triggers: true,
            record_collisions: true,
            max_recorded_collisions: 1,
            simulation_damping: 0.0,
            simulation_min_energy: 0.002,
            buoyancy_damping: 0.0,
            buoyancy_density: 1.0,
            buoyancy_resistance: 1.0,
        }
    }
}

impl RigidPhysicsConfig {
    #[inline]
    #[must_use]
    pub const fn enabled_initially(&self) -> bool {
        self.enabled_initially
    }

    #[inline]
    #[must_use]
    pub const fn collision_response_enabled(&self) -> bool {
        self.enable_collision_response
    }

    #[must_use]
    pub const fn use_mass(&self) -> bool {
        matches!(self.specify_mass_or_density, MassOrDensity::Mass)
    }

    #[must_use]
    pub const fn use_density(&self) -> bool {
        matches!(self.specify_mass_or_density, MassOrDensity::Density)
    }

    #[must_use]
    pub fn recorded_collision_capacity(&self) -> usize {
        if self.record_collisions {
            usize::try_from(self.max_recorded_collisions).unwrap_or(0)
        } else {
            0
        }
    }
}

/// Runtime rigid physics component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Physics/RigidPhysicsComponent.cpp:84`.
#[derive(
    AzRtti, Component, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "RigidPhysicsComponent",
    RIGID_PHYSICS_COMPONENT_TYPE_UUID,
    PhysicsComponent,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.RigidPhysicsComponent", version = 1)]
pub struct RigidPhysicsComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub physics_component: PhysicsComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: RigidPhysicsConfig,
}

impl RigidPhysicsComponent {
    #[inline]
    #[must_use]
    pub const fn enabled_initially(&self) -> bool {
        self.configuration.enabled_initially()
    }

    #[inline]
    #[must_use]
    pub const fn collision_response_enabled(&self) -> bool {
        self.configuration.collision_response_enabled()
    }
}

/// Reflected abstract base shared by legacy rigid and static physics components.
#[derive(AzRtti, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[az_rtti(
    name = "PhysicsComponent",
    PHYSICS_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
pub struct PhysicsComponent {
    #[serde(rename = "BaseClass1", default)]
    pub az_component: AzComponent,
}
