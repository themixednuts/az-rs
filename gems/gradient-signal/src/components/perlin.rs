use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::{GradientSampleParams, PerlinImprovedNoise};

/// Perlin gradient configuration.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/PerlinGradientComponent.h:40`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
pub struct PerlinGradientConfig {
    pub random_seed: i32,
    pub octave: i32,
    pub amplitude: f32,
    pub cache_results: bool,
    pub cache_resolution: u32,
    pub frequency: f32,
}

impl Default for PerlinGradientConfig {
    fn default() -> Self {
        Self {
            random_seed: 1,
            octave: 1,
            amplitude: 1.0,
            cache_results: false,
            cache_resolution: 0,
            frequency: 1.0,
        }
    }
}

impl PerlinGradientConfig {
    #[must_use]
    pub const fn normalized_random_seed(&self) -> i32 {
        if self.random_seed > 1 {
            self.random_seed
        } else {
            1
        }
    }

    #[must_use]
    pub fn sample_value(&self, params: GradientSampleParams) -> f32 {
        let noise = PerlinImprovedNoise::new(self.normalized_random_seed());
        self.sample_value_with_noise(&noise, params)
    }

    #[must_use]
    pub fn sample_value_with_noise(
        &self,
        noise: &PerlinImprovedNoise,
        params: GradientSampleParams,
    ) -> f32 {
        noise.generate_octave_noise(
            params.position.x,
            params.position.y,
            params.position.z,
            self.octave,
            self.amplitude,
            self.frequency,
        )
    }
}

/// Runtime Perlin gradient component.
///
/// Lumberyard reference: `dev/Gems/GradientSignal/Code/Source/Components/PerlinGradientComponent.h:48`.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.gradient_signal.PerlinGradientComponent", version = 1)]
pub struct PerlinGradientComponent {
    pub configuration: PerlinGradientConfig,
    #[reflect(ignore)]
    noise: PerlinImprovedNoise,
}

impl Default for PerlinGradientComponent {
    fn default() -> Self {
        Self::new(PerlinGradientConfig::default())
    }
}

impl PerlinGradientComponent {
    #[must_use]
    pub fn new(configuration: PerlinGradientConfig) -> Self {
        Self {
            noise: PerlinImprovedNoise::new(configuration.normalized_random_seed()),
            configuration,
        }
    }

    #[must_use]
    pub fn sample_value(&self, params: GradientSampleParams) -> f32 {
        if self.noise.seed() == self.configuration.normalized_random_seed() {
            self.configuration
                .sample_value_with_noise(&self.noise, params)
        } else {
            self.configuration.sample_value(params)
        }
    }
}
