use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::sampler::GradientSampler;

/// Invert gradient configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/InvertGradientComponent.h:34`.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
pub struct InvertGradientConfig {
    pub gradient: GradientSampler,
}

impl InvertGradientConfig {
    #[must_use]
    pub fn apply_invert(&self, sampled_value: f32) -> f32 {
        1.0 - sampled_value.clamp(0.0, 1.0)
    }
}

/// Runtime invert gradient component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/InvertGradientComponent.h:39`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.InvertGradientComponent", version = 1)]
pub struct InvertGradientComponent {
    pub configuration: InvertGradientConfig,
}
