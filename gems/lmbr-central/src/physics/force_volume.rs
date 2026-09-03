//! Reflected `LmbrCentral` force-volume component data.

use az_core::component::Component as AzComponent;
use az_derive::{AzRtti, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::{FORCE_VOLUME_COMPONENT_TYPE_UUID, FORCE_VOLUME_CONFIGURATION_TYPE_UUID};

/// Primary force branch used by the shipped compact force-volume component.
///
/// The native implementation branches on the exact integer values below. The
/// names match the `LmbrCentral` force concepts: a force away from the volume
/// center, or an authored direction.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceMode {
    Point = 0,
    #[default]
    Direction = 1,
}

impl TryFrom<i32> for ForceMode {
    type Error = UnknownForceMode;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Point),
            1 => Ok(Self::Direction),
            value => Err(UnknownForceMode(value)),
        }
    }
}

impl From<ForceMode> for i32 {
    fn from(value: ForceMode) -> Self {
        value as Self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown force-volume mode {0}")]
pub struct UnknownForceMode(pub i32);

/// Coordinate space for an authored directional force.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForceSpace {
    Local = 0,
    #[default]
    World = 1,
}

impl TryFrom<i32> for ForceSpace {
    type Error = UnknownForceSpace;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Local),
            1 => Ok(Self::World),
            value => Err(UnknownForceSpace(value)),
        }
    }
}

impl From<ForceSpace> for i32 {
    fn from(value: ForceSpace) -> Self {
        value as Self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("unknown force-volume space {0}")]
pub struct UnknownForceSpace(pub i32);

/// The compact force-volume configuration used by the current runtime.
///
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(
    name = "ForceVolumeConfiguration",
    FORCE_VOLUME_CONFIGURATION_TYPE_UUID
)]
pub struct ForceVolumeConfiguration {
    #[serde(rename = "Force Mode", default)]
    pub force_mode: ForceMode,
    #[serde(rename = "Force Space", default)]
    pub force_space: ForceSpace,
    #[serde(rename = "Force Apply", default)]
    pub force_mass_dependent: bool,
    #[serde(rename = "Force Scale", default)]
    pub force_scale: f32,
    #[serde(rename = "Force Direction", default)]
    pub force_direction: Vec3,
    #[serde(rename = "Volume Damping", default)]
    pub volume_damping: f32,
    #[serde(rename = "Volume Density", default)]
    pub volume_density: f32,
}

impl Default for ForceVolumeConfiguration {
    fn default() -> Self {
        // Defaults installed by the current ForceVolumeComponent descriptor.
        Self {
            force_mode: ForceMode::Direction,
            force_space: ForceSpace::World,
            force_mass_dependent: true,
            force_scale: 20.0,
            force_direction: Vec3::ONE,
            volume_damping: 0.0,
            volume_density: 0.0,
        }
    }
}

/// Runtime `ForceVolumeComponent` version 1.
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
    name = "ForceVolumeComponent",
    FORCE_VOLUME_COMPONENT_TYPE_UUID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.ForceVolumeComponent", version = 1)]
pub struct ForceVolumeComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: ForceVolumeConfiguration,
}
