//! Slope alignment modifier component data.

use az_gem_gradient_signal::GradientSampler;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::Mat3;
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptor;
use crate::math::normalized_or;

use super::stage::ModifierStage;

/// Gradient-backed slope alignment modifier configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SlopeAlignmentModifierComponent.h:36`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct SlopeAlignmentModifierConfig {
    pub allow_overrides: bool,
    pub range_min: f32,
    pub range_max: f32,
    pub gradient: GradientSampler,
}

impl Default for SlopeAlignmentModifierConfig {
    fn default() -> Self {
        Self {
            allow_overrides: false,
            range_min: 1.0,
            range_max: 1.0,
            gradient: GradientSampler::default(),
        }
    }
}

impl SlopeAlignmentModifierConfig {
    #[must_use]
    pub const fn modifier_stage(&self) -> ModifierStage {
        ModifierStage::Standard
    }

    #[must_use]
    pub const fn effective_range(&self, descriptor: Option<&VegetationDescriptor>) -> (f32, f32) {
        if let Some(descriptor) = descriptor
            && self.allow_overrides
            && descriptor.surface_alignment_override_enabled
        {
            return (
                descriptor.surface_alignment_min,
                descriptor.surface_alignment_max,
            );
        }

        (self.range_min, self.range_max)
    }

    #[must_use]
    pub fn alignment_factor_from_sample(
        &self,
        sample: f32,
        descriptor: Option<&VegetationDescriptor>,
    ) -> f32 {
        let (range_min, range_max) = self.effective_range(descriptor);
        (range_max - range_min).mul_add(sample, range_min)
    }

    #[must_use]
    pub fn alignment_from_sample(
        &self,
        normal: Vec3,
        sample: f32,
        descriptor: Option<&VegetationDescriptor>,
    ) -> Quat {
        Self::alignment_from_factor(
            normal,
            self.alignment_factor_from_sample(sample, descriptor),
        )
    }

    #[must_use]
    pub fn alignment_from_factor(normal: Vec3, factor: f32) -> Quat {
        let normal = normalized_or(normal, Vec3::Y);
        let up = normalized_or(Vec3::Y.lerp(normal, factor), Vec3::Y);
        let forward = normalized_or(Vec3::X.cross(up), Vec3::Z);
        let right = normalized_or(up.cross(forward), Vec3::X);

        Quat::from_mat3(&Mat3::from_cols(right, up, forward))
    }
}

/// Slope alignment modifier component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SlopeAlignmentModifierComponent.h:47`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.SlopeAlignmentModifierComponent", version = 1)]
pub struct SlopeAlignmentModifierComponent {
    pub configuration: SlopeAlignmentModifierConfig,
}
