//! Canonical `RockNRoll` character-controller authoring data.

use az_asset::UntypedAssetRef;
use az_core::{EntityId, component::Component as AzComponent};
use az_derive::{AzRtti, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{ShapeAssetRef, ShapeAssetReference, ShapeAssetReferenceConversionError};

pub const CHARACTER_DESC_TYPE_UUID: &str = "D03BD2CC-5E87-49A7-B490-47A6F6EBDC22";
pub const CHARACTER_CONTROLLER_CONFIG_TYPE_UUID: &str = "CC8AFEBD-5F7A-4E63-A472-FFC8591DB051";
pub const CHARACTER_CONTROLLER_COMPONENT_TYPE_UUID: &str = "E9D84B77-422A-4DCB-9EFF-708120B1B1A0";

/// Native `RockNRoll::CharacterDesc` version 1.
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(
    name = "RockNRoll::CharacterDesc",
    "D03BD2CC-5E87-49A7-B490-47A6F6EBDC22"
)]
pub struct CharacterDesc {
    #[serde(rename = "Up Direction", default)]
    pub up_direction: Vec3,
    #[serde(rename = "Max Slope", default)]
    pub max_slope: f32,
    #[serde(rename = "Contact distance", default)]
    pub contact_distance: f32,
    #[serde(rename = "Solver max iterations", default)]
    pub solver_max_iterations: u32,
    #[serde(rename = "Asynchronous", default)]
    pub asynchronous: bool,
}

impl Default for CharacterDesc {
    fn default() -> Self {
        // Native defaults validated against the current runtime.
        Self {
            up_direction: Vec3::Y,
            max_slope: core::f32::consts::FRAC_PI_2,
            contact_distance: 0.01,
            solver_max_iterations: 10,
            asynchronous: true,
        }
    }
}

/// Typed form of `CharacterControllerConfig::Shape Type` plus its payload.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterControllerShapeSource {
    /// Native discriminator zero: load the `RockNRoll` shape asset.
    Asset(ShapeAssetRef),
    /// Native discriminator one: consume the collider shape on another entity.
    Entity(EntityId),
}

impl Default for CharacterControllerShapeSource {
    fn default() -> Self {
        Self::Asset(ShapeAssetRef::empty())
    }
}

impl From<ShapeAssetRef> for CharacterControllerShapeSource {
    fn from(value: ShapeAssetRef) -> Self {
        Self::Asset(value)
    }
}

impl TryFrom<UntypedAssetRef> for CharacterControllerShapeSource {
    type Error = ShapeAssetReferenceConversionError;

    fn try_from(value: UntypedAssetRef) -> Result<Self, Self::Error> {
        ShapeAssetReference::try_from(value).map(|reference| Self::Asset(reference.into_inner()))
    }
}

impl From<EntityId> for CharacterControllerShapeSource {
    fn from(value: EntityId) -> Self {
        Self::Entity(value)
    }
}

/// Canonical typed character-controller configuration.
#[derive(AzTypeInfo, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(
    name = "CharacterControllerConfig",
    "CC8AFEBD-5F7A-4E63-A472-FFC8591DB051"
)]
pub struct CharacterControllerConfig {
    pub character: CharacterDesc,
    pub shape: CharacterControllerShapeSource,
    pub filter_name: String,
}

impl Default for CharacterControllerConfig {
    fn default() -> Self {
        // The native constructor selects asset discriminator zero, uses an
        // empty asset/id, and names the filter Static.
        Self {
            character: CharacterDesc::default(),
            shape: CharacterControllerShapeSource::default(),
            filter_name: "Static".to_owned(),
        }
    }
}

impl CharacterControllerConfig {
    #[inline]
    #[must_use]
    pub fn filter(&self) -> Option<&str> {
        let filter = self.filter_name.trim();
        (!filter.is_empty()).then_some(filter)
    }
}

/// Direct prefab character controller shared by client and server.
#[derive(
    AzRtti, Component, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "CharacterControllerComponent",
    "E9D84B77-422A-4DCB-9EFF-708120B1B1A0",
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.rock_n_roll.CharacterControllerComponent", version = 1)]
pub struct CharacterControllerComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: CharacterControllerConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parity test: every float here is compared against the exact literal the
    // native constructor assigns, so an epsilon would stop it noticing a
    // changed default.
    #[allow(clippy::float_cmp)]
    #[test]
    fn defaults_match_native_constructors() {
        let descriptor = CharacterDesc::default();
        assert_eq!(descriptor.up_direction, Vec3::Y);
        assert_eq!(descriptor.max_slope, core::f32::consts::FRAC_PI_2);
        assert_eq!(descriptor.contact_distance, 0.01);
        assert_eq!(descriptor.solver_max_iterations, 10);
        assert!(descriptor.asynchronous);

        let configuration = CharacterControllerConfig::default();
        assert!(matches!(
            configuration.shape,
            CharacterControllerShapeSource::Asset(ref asset) if asset.is_empty()
        ));
        assert_eq!(configuration.filter_name, "Static");
    }
}
