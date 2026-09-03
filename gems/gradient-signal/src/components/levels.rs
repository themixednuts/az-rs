use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::sampler::GradientSampler;
use crate::util::levels_value;

/// Levels gradient configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/LevelsGradientComponent.h:34`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct LevelsGradientConfig {
    pub gradient: GradientSampler,
    pub input_min: f32,
    pub input_mid: f32,
    pub input_max: f32,
    pub output_min: f32,
    pub output_max: f32,
}

impl Default for LevelsGradientConfig {
    fn default() -> Self {
        Self {
            gradient: GradientSampler::default(),
            input_min: 0.0,
            input_mid: 1.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
        }
    }
}

impl LevelsGradientConfig {
    #[must_use]
    pub fn apply_levels(&self, sampled_value: f32) -> f32 {
        levels_value(
            sampled_value,
            self.input_mid,
            self.input_min,
            self.input_max,
            self.output_min,
            self.output_max,
        )
    }
}

/// Runtime levels gradient component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/LevelsGradientComponent.h:44`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.LevelsGradientComponent", version = 1)]
pub struct LevelsGradientComponent {
    pub configuration: LevelsGradientConfig,
}
