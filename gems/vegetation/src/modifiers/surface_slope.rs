//! Surface slope filter component data.

use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptor;
use crate::instance::InstanceData;

use super::stage::FilterStage;

/// Surface slope filter configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SurfaceSlopeFilterComponent.h:27`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct SurfaceSlopeFilterConfig {
    pub filter_stage: FilterStage,
    pub allow_overrides: bool,
    pub slope_min_degrees: f32,
    pub slope_max_degrees: f32,
}

impl Default for SurfaceSlopeFilterConfig {
    fn default() -> Self {
        Self {
            filter_stage: FilterStage::Default,
            allow_overrides: false,
            slope_min_degrees: 0.0,
            slope_max_degrees: 180.0,
        }
    }
}

impl SurfaceSlopeFilterConfig {
    #[must_use]
    pub const fn effective_range(&self, descriptor: Option<&VegetationDescriptor>) -> (f32, f32) {
        if let Some(descriptor) = descriptor
            && self.allow_overrides
            && descriptor.slope_filter_override_enabled
        {
            return (descriptor.slope_filter_min, descriptor.slope_filter_max);
        }

        (self.slope_min_degrees, self.slope_max_degrees)
    }

    #[must_use]
    pub fn accepts_instance(
        &self,
        instance: &InstanceData,
        descriptor: Option<&VegetationDescriptor>,
    ) -> bool {
        let (range_min, range_max) = self.effective_range(descriptor);
        let min = range_min.min(range_max);
        let max = range_min.max(range_max);
        let c = instance.normal.normalize_or_zero().dot(Vec3::Y);
        let c1 = min.to_radians().cos();
        let c2 = max.to_radians().cos();
        c <= c1 && c >= c2
    }
}

/// Surface slope filter component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SurfaceSlopeFilterComponent.h:45`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.SurfaceSlopeFilterComponent", version = 1)]
pub struct SurfaceSlopeFilterComponent {
    pub configuration: SurfaceSlopeFilterConfig,
}
