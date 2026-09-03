//! Surface altitude filter component data.

use az_prefab::{Prefab, ReflectPrefab};
use bevy::ecs::entity::MapEntities;
use bevy::prelude::*;

use crate::descriptor::VegetationDescriptor;
use crate::instance::InstanceData;

use super::stage::FilterStage;

/// Surface altitude filter configuration.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SurfaceAltitudeFilterComponent.h:28`.
#[derive(Debug, Clone, PartialEq, Reflect, MapEntities)]
pub struct SurfaceAltitudeFilterConfig {
    pub filter_stage: FilterStage,
    pub allow_overrides: bool,
    #[entities]
    pub shape_entity: Option<Entity>,
    pub altitude_min: f32,
    pub altitude_max: f32,
}

impl Default for SurfaceAltitudeFilterConfig {
    fn default() -> Self {
        Self {
            filter_stage: FilterStage::Default,
            allow_overrides: false,
            shape_entity: None,
            altitude_min: 0.0,
            altitude_max: 128.0,
        }
    }
}

impl SurfaceAltitudeFilterConfig {
    #[must_use]
    pub const fn effective_range(&self, descriptor: Option<&VegetationDescriptor>) -> (f32, f32) {
        if let Some(descriptor) = descriptor
            && self.allow_overrides
            && descriptor.altitude_filter_override_enabled
        {
            return (
                descriptor.altitude_filter_min,
                descriptor.altitude_filter_max,
            );
        }

        (self.altitude_min, self.altitude_max)
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
        instance.position.y >= min && instance.position.y <= max
    }
}

/// Surface altitude filter component.
///
/// O3DE reference: `Gems/Vegetation/Code/Source/Components/SurfaceAltitudeFilterComponent.h:49`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect, MapEntities, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.vegetation.SurfaceAltitudeFilterComponent", version = 1)]
pub struct SurfaceAltitudeFilterComponent {
    #[entities]
    pub configuration: SurfaceAltitudeFilterConfig,
}
