use az_derive::AzRtti;
use az_gem_gradient_signal::{
    GradientLookup, GradientSampleParams, GradientSampler, NoGradientSources,
};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::constants::{
    FOREGROUND_LAYER, PRIORITY_MIN, VEGETATION_SPAWNER_COMPONENT_TYPE_ID,
    VEGETATION_SPAWNER_CONFIG_TYPE_ID,
};
use super::{ClaimContext, ClaimPoint};
use crate::descriptor::{
    DescriptorWeightSelectorConfig, VegetationDescriptor, VegetationDescriptorListConfig,
};
use crate::instance::InstanceData;
use crate::modifiers::{
    DistributionFilterConfig, FilterStage, ModifierStage, PositionModifierConfig,
    RotationModifierConfig, ScaleModifierConfig, SlopeAlignmentModifierConfig,
    SurfaceAltitudeFilterConfig, SurfaceMaskFilterConfig, SurfaceSlopeFilterConfig,
};

/// Base vegetation area configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/AreaComponentBase.h:38`.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct AreaConfig {
    pub layer: u32,
    pub priority: u32,
}

impl Default for AreaConfig {
    fn default() -> Self {
        Self {
            layer: FOREGROUND_LAYER,
            priority: PRIORITY_MIN,
        }
    }
}

/// Vegetation spawner component configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SpawnerComponent.h:47`.
#[derive(AzRtti, Component, Debug, Clone, PartialEq, Eq, Reflect)]
#[az_rtti(
    name = "Vegetation::SpawnerConfig",
    VEGETATION_SPAWNER_CONFIG_TYPE_ID,
    register
)]
#[reflect(Component)]
pub struct SpawnerConfig {
    pub area: AreaConfig,
    pub inherit_behavior: bool,
    pub use_relative_uvw: bool,
    pub allow_empty_meshes: bool,
    pub filter_stage: FilterStage,
}

impl Default for SpawnerConfig {
    fn default() -> Self {
        Self {
            area: AreaConfig::default(),
            inherit_behavior: true,
            use_relative_uvw: false,
            allow_empty_meshes: true,
            filter_stage: FilterStage::PreProcess,
        }
    }
}

/// Vegetation spawner component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SpawnerComponent.h:57`.
#[derive(AzRtti, Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Prefab)]
#[az_rtti(
    name = "Vegetation::SpawnerComponent",
    VEGETATION_SPAWNER_COMPONENT_TYPE_ID,
    register
)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.SpawnerComponent", version = 1)]
pub struct SpawnerComponent {
    pub configuration: SpawnerConfig,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnerFilterSet<'a> {
    pub distribution: Option<&'a DistributionFilterConfig>,
    pub altitude: Option<&'a SurfaceAltitudeFilterConfig>,
    pub surface_mask: Option<&'a SurfaceMaskFilterConfig>,
    pub slope: Option<&'a SurfaceSlopeFilterConfig>,
}

impl SpawnerFilterSet<'_> {
    fn accepts(
        &self,
        instance: &InstanceData,
        descriptor: &VegetationDescriptor,
        intended_stage: FilterStage,
        default_stage: FilterStage,
        gradients: &impl GradientLookup,
    ) -> bool {
        self.distribution.is_none_or(|filter| {
            filter_accepts_at(filter.filter_stage, intended_stage, default_stage, || {
                distribution_filter_accepts(filter, instance, gradients)
            })
        }) && self.altitude.is_none_or(|filter| {
            filter_accepts_at(filter.filter_stage, intended_stage, default_stage, || {
                filter.accepts_instance(instance, Some(descriptor))
            })
        }) && self.surface_mask.is_none_or(|filter| {
            filter_accepts_at(filter.filter_stage, intended_stage, default_stage, || {
                filter.accepts_instance(instance, Some(descriptor))
            })
        }) && self.slope.is_none_or(|filter| {
            filter_accepts_at(filter.filter_stage, intended_stage, default_stage, || {
                filter.accepts_instance(instance, Some(descriptor))
            })
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnerModifierSet<'a> {
    pub position: Option<&'a PositionModifierConfig>,
    pub rotation: Option<&'a RotationModifierConfig>,
    pub scale: Option<&'a ScaleModifierConfig>,
    pub slope_alignment: Option<&'a SlopeAlignmentModifierConfig>,
}

impl SpawnerModifierSet<'_> {
    fn apply(
        &self,
        instance: &mut InstanceData,
        descriptor: &VegetationDescriptor,
        gradients: &impl GradientLookup,
    ) {
        self.apply_stage(instance, descriptor, ModifierStage::PreProcess, gradients);
        self.apply_stage(instance, descriptor, ModifierStage::Standard, gradients);
        self.apply_stage(instance, descriptor, ModifierStage::PostProcess, gradients);
    }

    fn apply_stage(
        &self,
        instance: &mut InstanceData,
        descriptor: &VegetationDescriptor,
        stage: ModifierStage,
        gradients: &impl GradientLookup,
    ) {
        if let Some(position) = self.position
            && position.modifier_stage() == stage
        {
            let params = GradientSampleParams {
                position: instance.position,
            };
            let factors = Vec3::new(
                gradients.sample_gradient(&position.gradient_x, params),
                gradients.sample_gradient(&position.gradient_y, params),
                gradients.sample_gradient(&position.gradient_z, params),
            );
            instance.position += position.offset_from_factors(factors, Some(descriptor));
        }
        if let Some(rotation) = self.rotation
            && rotation.modifier_stage() == stage
        {
            let params = GradientSampleParams {
                position: instance.position,
            };
            let factors = Vec3::new(
                gradients.sample_gradient(&rotation.gradient_x, params),
                gradients.sample_gradient(&rotation.gradient_y, params),
                gradients.sample_gradient(&rotation.gradient_z, params),
            );
            instance.rotation = rotation.rotation_from_factors(factors, Some(descriptor));
        }
        if let Some(scale) = self.scale
            && scale.modifier_stage() == stage
        {
            let factor = sample_gradient_at(&scale.gradient, instance.position, gradients);
            instance.scale = scale.apply_scale(instance.scale, factor, Some(descriptor));
        }
        if let Some(slope_alignment) = self.slope_alignment
            && slope_alignment.modifier_stage() == stage
        {
            let sample =
                sample_gradient_at(&slope_alignment.gradient, instance.position, gradients);
            instance.alignment =
                slope_alignment.alignment_from_sample(instance.normal, sample, Some(descriptor));
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SpawnerProcessingSet<'a> {
    pub selector: Option<&'a DescriptorWeightSelectorConfig>,
    pub filters: SpawnerFilterSet<'a>,
    pub modifiers: SpawnerModifierSet<'a>,
}

impl SpawnerComponent {
    /// Claim available points with this spawner's descriptor list.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/Components/SpawnerComponent.cpp:427`.
    #[must_use]
    pub fn claim_positions(
        &self,
        descriptor_entity: Option<Entity>,
        context: &mut ClaimContext,
        descriptors: &VegetationDescriptorListConfig,
    ) -> Vec<InstanceData> {
        self.claim_positions_with_filters(
            descriptor_entity,
            context,
            descriptors,
            SpawnerFilterSet::default(),
        )
    }

    /// Claim available points with explicit instance filters.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/Components/SpawnerComponent.cpp:316`.
    #[must_use]
    pub fn claim_positions_with_filters(
        &self,
        descriptor_entity: Option<Entity>,
        context: &mut ClaimContext,
        descriptors: &VegetationDescriptorListConfig,
        filters: SpawnerFilterSet<'_>,
    ) -> Vec<InstanceData> {
        self.claim_positions_with_processing(
            descriptor_entity,
            context,
            descriptors,
            SpawnerProcessingSet {
                filters,
                ..Default::default()
            },
        )
    }

    /// Claim available points with explicit filters and modifiers.
    ///
    /// O3DE reference: `Gems/Vegetation/Code/Source/Components/SpawnerComponent.cpp:316`.
    #[must_use]
    pub fn claim_positions_with_processing(
        &self,
        descriptor_entity: Option<Entity>,
        context: &mut ClaimContext,
        descriptors: &VegetationDescriptorListConfig,
        processing: SpawnerProcessingSet<'_>,
    ) -> Vec<InstanceData> {
        self.claim_positions_with_gradient_sources(
            descriptor_entity,
            context,
            descriptors,
            processing,
            &NoGradientSources,
        )
    }

    pub(crate) fn claim_positions_with_gradient_sources(
        &self,
        descriptor_entity: Option<Entity>,
        context: &mut ClaimContext,
        descriptors: &VegetationDescriptorListConfig,
        processing: SpawnerProcessingSet<'_>,
        gradients: &impl GradientLookup,
    ) -> Vec<InstanceData> {
        let mut instances = Vec::new();
        let mut point_index = 0;

        while point_index < context.available_points.len() {
            let Some(instance) = self.claim_position(
                descriptor_entity,
                &context.available_points[point_index],
                descriptors,
                processing,
                gradients,
            ) else {
                point_index += 1;
                continue;
            };

            instances.push(instance);
            context.available_points.swap_remove(point_index);
        }

        instances
    }

    #[must_use]
    pub fn claim_position(
        &self,
        descriptor_entity: Option<Entity>,
        point: &ClaimPoint,
        descriptors: &VegetationDescriptorListConfig,
        processing: SpawnerProcessingSet<'_>,
        gradients: &impl GradientLookup,
    ) -> Option<InstanceData> {
        if let Some(selector) = processing.selector {
            return selector
                .select_descriptor_indices(
                    &descriptors.vegetation_descriptors,
                    point.position,
                    gradients,
                )
                .into_iter()
                .find_map(|index| {
                    let descriptor = descriptors.vegetation_descriptors.get(index)?;
                    self.process_instance(
                        descriptor_entity,
                        point,
                        index,
                        descriptor,
                        processing,
                        gradients,
                    )
                });
        }

        descriptors
            .vegetation_descriptors
            .iter()
            .enumerate()
            .find_map(|(index, descriptor)| {
                self.process_instance(
                    descriptor_entity,
                    point,
                    index,
                    descriptor,
                    processing,
                    gradients,
                )
            })
    }

    fn process_instance(
        &self,
        descriptor_entity: Option<Entity>,
        point: &ClaimPoint,
        descriptor_index: usize,
        descriptor: &VegetationDescriptor,
        processing: SpawnerProcessingSet<'_>,
        gradients: &impl GradientLookup,
    ) -> Option<InstanceData> {
        if !self.configuration.allow_empty_meshes && descriptor.has_empty_asset_references() {
            return None;
        }

        let mut instance = point.instance_data();
        instance.entity = descriptor_entity;
        instance.descriptor_index = Some(descriptor_index);
        if !processing.filters.accepts(
            &instance,
            descriptor,
            FilterStage::PreProcess,
            self.configuration.filter_stage,
            gradients,
        ) {
            return None;
        }
        processing
            .modifiers
            .apply(&mut instance, descriptor, gradients);
        processing
            .filters
            .accepts(
                &instance,
                descriptor,
                FilterStage::PostProcess,
                self.configuration.filter_stage,
                gradients,
            )
            .then_some(instance)
    }
}

fn distribution_filter_accepts(
    filter: &DistributionFilterConfig,
    instance: &InstanceData,
    gradients: &impl GradientLookup,
) -> bool {
    if filter.gradient.gradient.is_none() {
        return true;
    }

    filter.accepts_sample(sample_gradient_at(
        &filter.gradient,
        instance.position,
        gradients,
    ))
}

fn sample_gradient_at(
    sampler: &GradientSampler,
    position: Vec3,
    gradients: &impl GradientLookup,
) -> f32 {
    gradients.sample_gradient(sampler, GradientSampleParams { position })
}

const fn filter_runs_at(
    filter_stage: FilterStage,
    intended_stage: FilterStage,
    default_stage: FilterStage,
) -> bool {
    same_filter_stage(filter_stage, intended_stage)
        || (matches!(filter_stage, FilterStage::Default)
            && same_filter_stage(default_stage, intended_stage))
}

fn filter_accepts_at(
    filter_stage: FilterStage,
    intended_stage: FilterStage,
    default_stage: FilterStage,
    accepts: impl FnOnce() -> bool,
) -> bool {
    !filter_runs_at(filter_stage, intended_stage, default_stage) || accepts()
}

const fn same_filter_stage(left: FilterStage, right: FilterStage) -> bool {
    matches!(
        (left, right),
        (FilterStage::Default, FilterStage::Default)
            | (FilterStage::PreProcess, FilterStage::PreProcess)
            | (FilterStage::PostProcess, FilterStage::PostProcess)
    )
}
