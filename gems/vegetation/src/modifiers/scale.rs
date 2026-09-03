//! Scale modifier component data.

use az_gem_gradient_signal::GradientSampler;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptor;

use super::stage::ModifierStage;

/// Gradient-backed scale modifier configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/ScaleModifierComponent.h:36`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct ScaleModifierConfig {
    pub allow_overrides: bool,
    pub range_min: f32,
    pub range_max: f32,
    pub gradient: GradientSampler,
}

impl Default for ScaleModifierConfig {
    fn default() -> Self {
        Self {
            allow_overrides: false,
            range_min: 1.0,
            range_max: 1.0,
            gradient: GradientSampler::default(),
        }
    }
}

impl ScaleModifierConfig {
    #[must_use]
    pub const fn modifier_stage(&self) -> ModifierStage {
        ModifierStage::Standard
    }

    #[must_use]
    pub const fn effective_range(&self, descriptor: Option<&VegetationDescriptor>) -> (f32, f32) {
        if let Some(descriptor) = descriptor
            && self.allow_overrides
            && descriptor.scale_override_enabled
        {
            return (descriptor.scale_min, descriptor.scale_max);
        }

        (self.range_min, self.range_max)
    }

    #[must_use]
    pub fn scale_multiplier_from_factor(
        &self,
        factor: f32,
        descriptor: Option<&VegetationDescriptor>,
    ) -> f32 {
        let (range_min, range_max) = self.effective_range(descriptor);
        (range_max - range_min).mul_add(factor, range_min)
    }

    #[must_use]
    pub fn apply_scale(
        &self,
        base_scale: f32,
        factor: f32,
        descriptor: Option<&VegetationDescriptor>,
    ) -> f32 {
        (base_scale * self.scale_multiplier_from_factor(factor, descriptor)).max(0.01)
    }
}

/// Scale modifier component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/ScaleModifierComponent.h:47`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.ScaleModifierComponent", version = 1)]
pub struct ScaleModifierComponent {
    pub configuration: ScaleModifierConfig,
}
