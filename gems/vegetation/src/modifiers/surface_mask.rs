//! Surface mask filter component data.

use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::descriptor::{OverrideMode, VegetationDescriptor};
use crate::instance::InstanceData;
use crate::surface::{
    VegetationSurfaceTag, has_matching_surface_tag_weight, has_valid_surface_tags,
};

use super::stage::FilterStage;

/// Surface mask filter configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SurfaceMaskFilterComponent.h:34`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct SurfaceMaskFilterConfig {
    pub filter_stage: FilterStage,
    pub allow_overrides: bool,
    pub inclusive_surface_masks: Vec<VegetationSurfaceTag>,
    pub inclusive_weight_min: f32,
    pub inclusive_weight_max: f32,
    pub exclusive_surface_masks: Vec<VegetationSurfaceTag>,
    pub exclusive_weight_min: f32,
    pub exclusive_weight_max: f32,
}

impl Default for SurfaceMaskFilterConfig {
    fn default() -> Self {
        Self {
            filter_stage: FilterStage::Default,
            allow_overrides: false,
            inclusive_surface_masks: Vec::new(),
            inclusive_weight_min: 0.1,
            inclusive_weight_max: 1.0,
            exclusive_surface_masks: Vec::new(),
            exclusive_weight_min: 0.1,
            exclusive_weight_max: 1.0,
        }
    }
}

impl SurfaceMaskFilterConfig {
    #[must_use]
    pub fn accepts_instance(
        &self,
        instance: &InstanceData,
        descriptor: Option<&VegetationDescriptor>,
    ) -> bool {
        let use_component_tags = !self.allow_overrides
            || descriptor.is_some_and(|descriptor| {
                descriptor.surface_filter_override_mode != OverrideMode::Replace
            });
        let use_descriptor_tags = self.allow_overrides
            && descriptor.is_some_and(|descriptor| {
                descriptor.surface_filter_override_mode != OverrideMode::Disable
            });

        if use_component_tags
            && has_matching_surface_tag_weight(
                &instance.masks,
                &self.exclusive_surface_masks,
                self.exclusive_weight_min,
                self.exclusive_weight_max,
            )
        {
            return false;
        }

        if let Some(descriptor) = descriptor {
            if use_descriptor_tags
                && has_matching_surface_tag_weight(
                    &instance.masks,
                    &descriptor.exclusive_surface_filter_tags,
                    self.exclusive_weight_min,
                    self.exclusive_weight_max,
                )
            {
                return false;
            }

            if use_descriptor_tags
                && has_matching_surface_tag_weight(
                    &instance.masks,
                    &descriptor.inclusive_surface_filter_tags,
                    self.inclusive_weight_min,
                    self.inclusive_weight_max,
                )
            {
                return true;
            }
        }

        if use_component_tags
            && has_matching_surface_tag_weight(
                &instance.masks,
                &self.inclusive_surface_masks,
                self.inclusive_weight_min,
                self.inclusive_weight_max,
            )
        {
            return true;
        }

        (!use_component_tags || !has_valid_surface_tags(&self.inclusive_surface_masks))
            && (!use_descriptor_tags
                || descriptor.is_none_or(|descriptor| {
                    !has_valid_surface_tags(&descriptor.inclusive_surface_filter_tags)
                }))
    }
}

/// Surface mask filter component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SurfaceMaskFilterComponent.h:55`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.SurfaceMaskFilterComponent", version = 1)]
pub struct SurfaceMaskFilterComponent {
    pub configuration: SurfaceMaskFilterConfig,
}
