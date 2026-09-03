use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::sampler::GradientSampler;

/// Threshold gradient configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/ThresholdGradientComponent.h:34`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct ThresholdGradientConfig {
    pub gradient: GradientSampler,
    pub threshold: f32,
}

impl Default for ThresholdGradientConfig {
    fn default() -> Self {
        Self {
            gradient: GradientSampler::default(),
            threshold: 0.5,
        }
    }
}

impl ThresholdGradientConfig {
    #[must_use]
    pub fn apply_threshold(&self, sampled_value: f32) -> f32 {
        if sampled_value <= self.threshold {
            0.0
        } else {
            1.0
        }
    }
}

/// Runtime threshold gradient component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/ThresholdGradientComponent.h:40`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.ThresholdGradientComponent", version = 1)]
pub struct ThresholdGradientComponent {
    pub configuration: ThresholdGradientConfig,
}
