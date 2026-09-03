//! `RockNRoll` world-system component and native defaults.

use az_core::component::Component as AzComponent;
use az_derive::AzRtti;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const ROCK_N_ROLL_SYSTEM_COMPONENT_TYPE_UUID: &str = "0518905D-AD60-4818-9222-EDF2FEB7AE7D";

/// Controls construction of the process-wide default `RockNRoll` world.
#[derive(
    AzRtti, Component, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "RockNRollSystemComponent",
    "0518905D-AD60-4818-9222-EDF2FEB7AE7D",
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.rock_n_roll.RockNRollSystemComponent", version = 1)]
pub struct RockNRollSystemComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "TrackAllocationRecords", default)]
    pub track_allocation_records: bool,
    #[serde(rename = "CreateDefaultWorld", default)]
    pub create_default_world: bool,
    #[serde(rename = "DefaultGravity", default)]
    pub default_gravity: Vec3,
}

impl Default for RockNRollSystemComponent {
    fn default() -> Self {
        // Native defaults enable the default world, disable allocation
        // tracking, and use Earth-normal downward gravity.
        Self {
            az_component: AzComponent::default(),
            track_allocation_records: false,
            create_default_world: true,
            default_gravity: Vec3::new(0.0, 0.0, -9.81),
        }
    }
}
