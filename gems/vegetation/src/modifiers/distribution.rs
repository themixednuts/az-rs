//! Distribution filter component data.

use az_gem_gradient_signal::GradientSampler;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use super::stage::FilterStage;

/// Gradient-backed distribution filter configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/DistributionFilterComponent.h:36`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct DistributionFilterConfig {
    pub filter_stage: FilterStage,
    pub threshold_min: f32,
    pub threshold_max: f32,
    pub gradient: GradientSampler,
}

impl Default for DistributionFilterConfig {
    fn default() -> Self {
        Self {
            filter_stage: FilterStage::Default,
            threshold_min: 0.1,
            threshold_max: 1.0,
            gradient: GradientSampler::default(),
        }
    }
}

impl DistributionFilterConfig {
    #[must_use]
    pub fn accepts_sample(&self, sample: f32) -> bool {
        sample >= self.threshold_min && sample <= self.threshold_max
    }
}

/// Distribution filter component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/DistributionFilterComponent.h:49`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.DistributionFilterComponent", version = 1)]
pub struct DistributionFilterComponent {
    pub configuration: DistributionFilterConfig,
}
