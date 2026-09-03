use bevy::prelude::*;

use super::VegetationSurfaceTag;
use super::constants::{
    DEFAULT_LOWER_SURFACE_DISTANCE_METERS, DEFAULT_UPPER_SURFACE_DISTANCE_METERS,
};

/// Distance range from one or more comparison surface tags.
///
/// O3DE reference: `Gems/Vegetation/Code/Include/Vegetation/Descriptor.h:28`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct VegetationSurfaceTagDistance {
    pub tags: Vec<VegetationSurfaceTag>,
    pub upper_distance_in_meters: f32,
    pub lower_distance_in_meters: f32,
}

impl Default for VegetationSurfaceTagDistance {
    fn default() -> Self {
        Self {
            tags: Vec::new(),
            upper_distance_in_meters: DEFAULT_UPPER_SURFACE_DISTANCE_METERS,
            lower_distance_in_meters: DEFAULT_LOWER_SURFACE_DISTANCE_METERS,
        }
    }
}
