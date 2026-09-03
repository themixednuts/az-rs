//! Rotation modifier component data.

use az_gem_gradient_signal::GradientSampler;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::EulerRot;
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptor;

use super::stage::ModifierStage;

/// Gradient-backed rotation modifier configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/RotationModifierComponent.h:37`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct RotationModifierConfig {
    pub allow_overrides: bool,
    pub range_min_degrees: Vec3,
    pub range_max_degrees: Vec3,
    pub gradient_x: GradientSampler,
    pub gradient_y: GradientSampler,
    pub gradient_z: GradientSampler,
}

impl Default for RotationModifierConfig {
    fn default() -> Self {
        Self {
            allow_overrides: false,
            range_min_degrees: Vec3::new(0.0, -180.0, 0.0),
            range_max_degrees: Vec3::new(0.0, 180.0, 0.0),
            gradient_x: GradientSampler::default(),
            gradient_y: GradientSampler::default(),
            gradient_z: GradientSampler::default(),
        }
    }
}

impl RotationModifierConfig {
    #[must_use]
    pub const fn modifier_stage(&self) -> ModifierStage {
        ModifierStage::Standard
    }

    #[must_use]
    pub const fn effective_range(&self, descriptor: Option<&VegetationDescriptor>) -> (Vec3, Vec3) {
        if let Some(descriptor) = descriptor
            && self.allow_overrides
            && descriptor.rotation_override_enabled
        {
            return (
                descriptor.rotation_min_degrees,
                descriptor.rotation_max_degrees,
            );
        }

        (self.range_min_degrees, self.range_max_degrees)
    }

    #[must_use]
    pub fn euler_degrees_from_factors(
        &self,
        factors: Vec3,
        descriptor: Option<&VegetationDescriptor>,
    ) -> Vec3 {
        let (range_min, range_max) = self.effective_range(descriptor);
        range_min + (range_max - range_min) * factors
    }

    #[must_use]
    pub fn rotation_from_factors(
        &self,
        factors: Vec3,
        descriptor: Option<&VegetationDescriptor>,
    ) -> Quat {
        let degrees = self.euler_degrees_from_factors(factors, descriptor);
        Quat::from_euler(
            EulerRot::XYZ,
            degrees.x.to_radians(),
            degrees.y.to_radians(),
            degrees.z.to_radians(),
        )
    }
}

/// Rotation modifier component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/RotationModifierComponent.h:53`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.RotationModifierComponent", version = 1)]
pub struct RotationModifierComponent {
    pub configuration: RotationModifierConfig,
}
