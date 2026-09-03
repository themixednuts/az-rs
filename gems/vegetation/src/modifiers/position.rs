//! Position modifier component data.

use az_gem_gradient_signal::GradientSampler;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptor;

use super::stage::ModifierStage;

/// Gradient-backed position modifier configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/PositionModifierComponent.h:46`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct PositionModifierConfig {
    pub allow_overrides: bool,
    pub range_min: Vec3,
    pub range_max: Vec3,
    pub gradient_x: GradientSampler,
    pub gradient_y: GradientSampler,
    pub gradient_z: GradientSampler,
    pub align_to_water: bool,
    pub water_height_offset: f32,
}

impl Default for PositionModifierConfig {
    fn default() -> Self {
        Self {
            allow_overrides: false,
            range_min: Vec3::new(-0.3, 0.0, -0.3),
            range_max: Vec3::new(0.3, 0.0, 0.3),
            gradient_x: GradientSampler::default(),
            gradient_y: GradientSampler::default(),
            gradient_z: GradientSampler::default(),
            align_to_water: false,
            water_height_offset: 0.0,
        }
    }
}

impl PositionModifierConfig {
    #[must_use]
    pub const fn modifier_stage(&self) -> ModifierStage {
        ModifierStage::PreProcess
    }

    #[must_use]
    pub const fn effective_range(&self, descriptor: Option<&VegetationDescriptor>) -> (Vec3, Vec3) {
        if let Some(descriptor) = descriptor
            && self.allow_overrides
            && descriptor.position_override_enabled
        {
            return (descriptor.position_min, descriptor.position_max);
        }

        (self.range_min, self.range_max)
    }

    #[must_use]
    pub fn offset_from_factors(
        &self,
        factors: Vec3,
        descriptor: Option<&VegetationDescriptor>,
    ) -> Vec3 {
        let (range_min, range_max) = self.effective_range(descriptor);
        range_min + (range_max - range_min) * factors
    }
}

/// Position modifier component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/PositionModifierComponent.h:62`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.PositionModifierComponent", version = 1)]
pub struct PositionModifierComponent {
    pub configuration: PositionModifierConfig,
}
