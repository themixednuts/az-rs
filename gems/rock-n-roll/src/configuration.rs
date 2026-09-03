//! Canonical `RockNRoll` rigid-body prefab and authoring configuration.
//!
//! The legacy reflected record was flat. The Azoth component keeps the same
//! values and source defaults, but groups them by meaning so RON is typed and
//! editable without string-key metadata.

use std::borrow::Borrow;

use az_asset::UntypedAssetRef;
use az_core::EntityId;
use az_derive::{AzRtti, AzTypeInfo};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    ContactMaterial, ContinuousPhysicsMode, Damping, ShapeAssetRef, ShapeAssetReference,
    ShapeAssetReferenceConversionError, SleepConfiguration, SleepConfigurationError,
};

pub const RIGID_BODY_COMPONENT_TYPE_UUID: &str = "51F92E5E-BD1A-4F9B-89F7-174205E4CBC7";
pub const RIGID_BODY_CONFIGURATION_TYPE_UUID: &str = "96C23E11-A0CB-43F9-9554-73470CF201A3";

/// Source selected by the reflected `Shape Type` discriminator.
#[derive(Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigidBodyShapeSource {
    Asset(ShapeAssetRef),
    Entity(EntityId),
}

impl Default for RigidBodyShapeSource {
    fn default() -> Self {
        Self::Asset(ShapeAssetRef::empty())
    }
}

impl From<ShapeAssetRef> for RigidBodyShapeSource {
    fn from(value: ShapeAssetRef) -> Self {
        Self::Asset(value)
    }
}

impl TryFrom<UntypedAssetRef> for RigidBodyShapeSource {
    type Error = ShapeAssetReferenceConversionError;

    fn try_from(value: UntypedAssetRef) -> Result<Self, Self::Error> {
        ShapeAssetReference::try_from(value).map(|reference| Self::Asset(reference.into_inner()))
    }
}

impl From<EntityId> for RigidBodyShapeSource {
    fn from(value: EntityId) -> Self {
        Self::Entity(value)
    }
}

/// Recovered numeric `RockNRoll` `MotionType` value.
///
/// `SerializeContext` exposes the authoring field as `Physics behavior`, while
/// `RigidBodyDriller` identifies the native body field as `MotionType`. Both
/// surfaces expose an `s32`; neither supplies the original value labels. A
/// transparent type prevents accidental mixing with other integers without
/// inventing names.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Reflect, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct MotionType(i32);

impl MotionType {
    #[inline]
    #[must_use]
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    #[inline]
    #[must_use]
    pub const fn value(self) -> i32 {
        self.0
    }

    /// Whether construction takes the pose-only fallback used by value zero
    /// and by values outside the recovered `1..=4` range.
    #[inline]
    #[must_use]
    pub const fn uses_pose_only_state(self) -> bool {
        !matches!(self.0, 1..=4)
    }

    /// Whether construction points at the process-wide zero-state block.
    #[inline]
    #[must_use]
    pub const fn uses_shared_zero_state(self) -> bool {
        self.0 == 1
    }

    /// Whether the shipping body constructor allocates pose history and the
    /// `0xc0` dynamics-state record for this value.
    #[inline]
    #[must_use]
    pub const fn has_pose_history_and_dynamics_state(self) -> bool {
        matches!(self.0, 2..=4)
    }

    /// Whether the allocated dynamics state has zero inverse mass and zero
    /// inverse inertia in the shipping body constructor.
    #[inline]
    #[must_use]
    pub const fn has_zero_inverse_mass_and_inertia(self) -> bool {
        matches!(self.0, 2 | 3)
    }

    /// Whether the shipping constructor derives or copies nonzero inverse mass
    /// and inverse inertia into the body dynamics state.
    #[inline]
    #[must_use]
    pub const fn has_dynamic_mass_properties(self) -> bool {
        self.0 == 4
    }

    /// Whether the ordered continuous-collision loop advances this body through
    /// the remaining substep after resolving a queued collision fraction.
    #[inline]
    #[must_use]
    pub const fn participates_in_continuous_pose_integration(self) -> bool {
        matches!(self.0, 2 | 4)
    }

    /// Whether the ordinary substep walks this body through the dedicated
    /// intrusive list and integrates it independently of an island.
    #[inline]
    #[must_use]
    pub const fn participates_in_unconstrained_substep_integration(self) -> bool {
        self.0 == 2
    }
}

impl From<i32> for MotionType {
    fn from(value: i32) -> Self {
        Self::new(value)
    }
}

impl From<MotionType> for i32 {
    fn from(value: MotionType) -> Self {
        value.value()
    }
}

impl AsRef<i32> for MotionType {
    fn as_ref(&self) -> &i32 {
        &self.0
    }
}

impl Borrow<i32> for MotionType {
    fn borrow(&self) -> &i32 {
        &self.0
    }
}

/// Optional material lookup plus the coefficients used when lookup fails.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Default)]
pub struct RigidBodyMaterialConfiguration {
    pub name: String,
    pub coefficients: ContactMaterial,
}

impl AsRef<ContactMaterial> for RigidBodyMaterialConfiguration {
    fn as_ref(&self) -> &ContactMaterial {
        &self.coefficients
    }
}

impl Borrow<ContactMaterial> for RigidBodyMaterialConfiguration {
    fn borrow(&self) -> &ContactMaterial {
        &self.coefficients
    }
}

/// Per-body values authored beside `RockNRoll`'s numeric continuous mode.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ContinuousBodyConfiguration {
    pub mode: ContinuousPhysicsMode,
    pub distance_factor: f32,
    pub sphere_radius: f32,
}

impl ContinuousBodyConfiguration {
    /// Validates the two authored scalars without assigning semantics that the
    /// native descriptor converter did not use.
    ///
    /// # Errors
    ///
    /// Returns [`RigidBodyConfigurationError::NonFiniteScalar`] if either
    /// `distance_factor` or `sphere_radius` is `NaN` or infinite.
    pub fn validate(self) -> Result<(), RigidBodyConfigurationError> {
        for (field, value) in [
            ("continuous distance factor", self.distance_factor),
            ("continuous sphere radius", self.sphere_radius),
        ] {
            if !value.is_finite() {
                return Err(RigidBodyConfigurationError::NonFiniteScalar { field, value });
            }
        }
        Ok(())
    }
}

impl Default for ContinuousBodyConfiguration {
    fn default() -> Self {
        Self {
            mode: ContinuousPhysicsMode::Disabled,
            distance_factor: 0.3,
            sphere_radius: 1.0,
        }
    }
}

/// Canonical typed form of the reflected `RockNRoll` rigid-body record.
#[derive(AzTypeInfo, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(
    name = "RigidBodyConfiguration",
    "96C23E11-A0CB-43F9-9554-73470CF201A3"
)]
pub struct RigidBodyConfiguration {
    pub shape: RigidBodyShapeSource,
    pub material: RigidBodyMaterialConfiguration,
    pub motion_type: MotionType,
    pub mass: f32,
    pub initially_active: bool,
    pub initial_linear_velocity: Vec3,
    pub initial_angular_velocity: Vec3,
    pub damping: Damping,
    pub sleep: SleepConfiguration,
    pub continuous: ContinuousBodyConfiguration,
    pub auto_inertia_tensor: bool,
}

impl RigidBodyConfiguration {
    /// Validates values consumed by body materialization.
    ///
    /// # Errors
    ///
    /// Returns [`RigidBodyConfigurationError::InvalidMass`] if `mass` is not
    /// finite and positive, [`RigidBodyConfigurationError::NonFiniteVector`] if
    /// either initial velocity is not finite,
    /// [`RigidBodyConfigurationError::Dynamics`],
    /// [`RigidBodyConfigurationError::Material`] or
    /// [`RigidBodyConfigurationError::Sleep`] if the damping, contact-material
    /// or sleep values are out of range, and any error
    /// [`ContinuousBodyConfiguration::validate`] returns.
    pub fn validate(&self) -> Result<(), RigidBodyConfigurationError> {
        if !self.mass.is_finite() || self.mass <= 0.0 {
            return Err(RigidBodyConfigurationError::InvalidMass(self.mass));
        }
        for (field, value) in [
            ("initial linear velocity", self.initial_linear_velocity),
            ("initial angular velocity", self.initial_angular_velocity),
        ] {
            if !value.is_finite() {
                return Err(RigidBodyConfigurationError::NonFiniteVector { field, value });
            }
        }
        Damping::try_new(self.damping.linear, self.damping.angular)?;
        ContactMaterial::try_new(
            self.material.coefficients.friction(),
            self.material.coefficients.restitution(),
        )?;
        SleepConfiguration::try_new(
            self.sleep.condition,
            self.sleep.linear_velocity_threshold,
            self.sleep.angular_velocity_threshold,
            self.sleep.energy_threshold,
            self.sleep.required_duration,
        )?;
        self.continuous.validate()
    }

    /// Sleep values actually passed by the shipping component converter.
    ///
    /// The converter copies authored sleep energy into both native energy and
    /// duration slots. Keeping this quirk explicit preserves 1:1 behavior
    /// while retaining the separately authored duration for editing.
    #[must_use]
    pub const fn runtime_sleep_configuration(&self) -> SleepConfiguration {
        SleepConfiguration {
            required_duration: self.sleep.energy_threshold,
            ..self.sleep
        }
    }
}

impl Default for RigidBodyConfiguration {
    fn default() -> Self {
        Self {
            shape: RigidBodyShapeSource::default(),
            material: RigidBodyMaterialConfiguration::default(),
            motion_type: MotionType::default(),
            mass: 1.0,
            initially_active: true,
            initial_linear_velocity: Vec3::ZERO,
            initial_angular_velocity: Vec3::ZERO,
            damping: Damping {
                linear: 0.05,
                angular: 0.15,
            },
            sleep: SleepConfiguration::default(),
            continuous: ContinuousBodyConfiguration::default(),
            auto_inertia_tensor: false,
        }
    }
}

/// Direct prefab component used by editor, client, and authoritative server.
#[derive(
    AzRtti, Component, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize, Prefab,
)]
#[az_rtti(
    name = "RigidBodyComponent",
    "51F92E5E-BD1A-4F9B-89F7-174205E4CBC7",
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.rock_n_roll.RigidBodyComponent", version = 1)]
pub struct RigidBodyComponent {
    pub configuration: RigidBodyConfiguration,
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum RigidBodyConfigurationError {
    #[error("rigid-body mass must be finite and positive, got {0}")]
    InvalidMass(f32),
    #[error("{field} must be finite, got {value:?}")]
    NonFiniteVector { field: &'static str, value: Vec3 },
    #[error("{field} must be finite, got {value}")]
    NonFiniteScalar { field: &'static str, value: f32 },
    #[error(transparent)]
    Dynamics(#[from] crate::DynamicsError),
    #[error(transparent)]
    Material(#[from] crate::ContactMaterialError),
    #[error(transparent)]
    Sleep(#[from] SleepConfigurationError),
}

#[cfg(test)]
mod tests {
    use super::*;

    // Parity test: every float here is compared against the exact literal the
    // native constructor assigns, so an epsilon would stop it noticing a
    // changed default.
    #[allow(clippy::float_cmp)]
    #[test]
    fn defaults_match_the_concrete_native_constructor() {
        let configuration = RigidBodyConfiguration::default();

        assert!(matches!(
            configuration.shape,
            RigidBodyShapeSource::Asset(ref asset) if asset.is_empty()
        ));
        assert_eq!(configuration.motion_type.value(), 0);
        assert_eq!(configuration.mass, 1.0);
        assert!(configuration.initially_active);
        assert_eq!(configuration.material.coefficients.friction(), 0.5);
        assert_eq!(configuration.material.coefficients.restitution(), 0.5);
        assert_eq!(configuration.damping.linear, 0.05);
        assert_eq!(configuration.damping.angular, 0.15);
        assert_eq!(configuration.sleep, SleepConfiguration::default());
        assert_eq!(
            configuration.continuous.mode,
            ContinuousPhysicsMode::Disabled
        );
        assert_eq!(configuration.continuous.distance_factor, 0.3);
        assert_eq!(configuration.continuous.sphere_radius, 1.0);
        assert!(!configuration.auto_inertia_tensor);
        assert!(configuration.validate().is_ok());
    }

    #[test]
    fn motion_type_queries_match_native_state_construction() {
        for raw in -1..=5 {
            let motion_type = MotionType::new(raw);
            assert_eq!(motion_type.uses_pose_only_state(), !(1..=4).contains(&raw));
            assert_eq!(motion_type.uses_shared_zero_state(), raw == 1);
            assert_eq!(
                motion_type.has_pose_history_and_dynamics_state(),
                matches!(raw, 2..=4)
            );
            assert_eq!(
                motion_type.has_zero_inverse_mass_and_inertia(),
                matches!(raw, 2 | 3)
            );
            assert_eq!(motion_type.has_dynamic_mass_properties(), raw == 4);
            assert_eq!(
                motion_type.participates_in_continuous_pose_integration(),
                matches!(raw, 2 | 4)
            );
            assert_eq!(
                motion_type.participates_in_unconstrained_substep_integration(),
                raw == 2
            );
        }
    }

    // The quirk under test is that the converter copies authored values
    // verbatim into other slots, so the assertions must be bit-exact against
    // the constants passed to `try_new`.
    #[allow(clippy::float_cmp)]
    #[test]
    fn runtime_sleep_preserves_the_shipping_converter_quirk() {
        let configuration = RigidBodyConfiguration {
            sleep: SleepConfiguration::try_new(
                crate::SleepCondition::SmoothedEnergy,
                0.8,
                1.0,
                0.25,
                9.0,
            )
            .unwrap(),
            ..Default::default()
        };

        assert_eq!(
            configuration.runtime_sleep_configuration().energy_threshold,
            0.25
        );
        assert_eq!(
            configuration
                .runtime_sleep_configuration()
                .required_duration,
            0.25
        );
        assert_eq!(configuration.sleep.required_duration, 9.0);
    }

    #[test]
    fn az_identity_matches_the_recovered_types() {
        assert_eq!(
            <RigidBodyConfiguration as az_core::AzTypeInfo>::TYPE_ID
                .to_string()
                .to_uppercase(),
            RIGID_BODY_CONFIGURATION_TYPE_UUID
        );
        assert_eq!(
            <RigidBodyComponent as az_core::AzTypeInfo>::TYPE_ID
                .to_string()
                .to_uppercase(),
            RIGID_BODY_COMPONENT_TYPE_UUID
        );
        assert_eq!(
            <RigidBodyComponent as az_core::AzRtti>::BASE_TYPE_IDS,
            &[<az_core::component::Component as az_core::AzTypeInfo>::TYPE_ID]
        );
    }
}
