//! Descriptor weight selector component data.

use std::cmp::Ordering;

use az_gem_gradient_signal::{GradientLookup, GradientSampleParams, GradientSampler};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;

use super::descriptor::VegetationDescriptor;

/// Descriptor weight sort behavior.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Ebuses/DescriptorWeightSelectorRequestBus.h:23`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
pub enum SortBehavior {
    #[default]
    Unsorted,
    Ascending,
    Descending,
}

impl SortBehavior {
    #[must_use]
    pub const fn from_native_value(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Unsorted),
            1 => Some(Self::Ascending),
            2 => Some(Self::Descending),
            _ => None,
        }
    }
}

/// Gradient-backed descriptor selector configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/DescriptorWeightSelectorComponent.h:29`.
#[derive(Debug, Clone, PartialEq, Reflect, MapEntities)]
pub struct DescriptorWeightSelectorConfig {
    pub sort_behavior: SortBehavior,
    #[entities]
    pub gradient: GradientSampler,
}

impl Default for DescriptorWeightSelectorConfig {
    fn default() -> Self {
        Self {
            sort_behavior: SortBehavior::Unsorted,
            gradient: GradientSampler::default(),
        }
    }
}

impl DescriptorWeightSelectorConfig {
    /// Return descriptor indices retained by this selector.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/Components/DescriptorWeightSelectorComponent.cpp:137`.
    #[must_use]
    pub fn select_descriptor_indices(
        &self,
        descriptors: &[VegetationDescriptor],
        position: Vec3,
        gradients: &impl GradientLookup,
    ) -> Vec<usize> {
        let mut indices = (0..descriptors.len()).collect::<Vec<_>>();
        match self.sort_behavior {
            SortBehavior::Unsorted => {}
            SortBehavior::Ascending => {
                indices.sort_unstable_by(|left, right| {
                    compare_descriptor_weights(
                        descriptors[*left].weight,
                        descriptors[*right].weight,
                    )
                });
            }
            SortBehavior::Descending => {
                indices.sort_unstable_by(|left, right| {
                    compare_descriptor_weights(
                        descriptors[*right].weight,
                        descriptors[*left].weight,
                    )
                });
            }
        }

        let total_weight = indices
            .iter()
            .map(|index| descriptors[*index].weight)
            .sum::<f32>();
        let sample = gradients.sample_gradient(&self.gradient, GradientSampleParams { position });
        let minimum_weight = sample * total_weight;
        let mut current_weight = 0.0;
        let mut skip_count = 0;
        for index in &indices {
            current_weight += descriptors[*index].weight;
            if current_weight < minimum_weight {
                skip_count += 1;
            }
        }

        if skip_count > 0 {
            indices.drain(..skip_count);
        }
        indices
    }
}

/// Descriptor weight selector component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/DescriptorWeightSelectorComponent.h:46`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(
    tag = "azoth.vegetation.DescriptorWeightSelectorComponent",
    version = 1
)]
pub struct DescriptorWeightSelectorComponent {
    #[entities]
    pub configuration: DescriptorWeightSelectorConfig,
}

fn compare_descriptor_weights(left: f32, right: f32) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}
