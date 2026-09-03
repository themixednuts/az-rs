use bevy::math::Vec3A;
use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;

use super::constants::{FOREGROUND_LAYER, PRIORITY_MIN};

/// Registered vegetation area metadata.
#[derive(Component, Debug, Clone, PartialEq, Reflect)]
#[reflect(Component)]
pub struct VegetationAreaInfo {
    pub entity: Option<Entity>,
    pub bounds: Aabb3d,
    pub layer: u32,
    pub priority: u32,
}

impl Default for VegetationAreaInfo {
    fn default() -> Self {
        Self {
            entity: None,
            bounds: Aabb3d::from_min_max(Vec3A::ZERO, Vec3A::ZERO),
            layer: FOREGROUND_LAYER,
            priority: PRIORITY_MIN,
        }
    }
}
