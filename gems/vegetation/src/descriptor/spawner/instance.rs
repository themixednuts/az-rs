use bevy::prelude::*;

use crate::non_empty_path;

use super::{
    DynamicSliceInstanceSpawner, EmptyInstanceSpawner, InstanceSpawnerKind,
    LegacyVegetationInstanceSpawner,
};

/// Vegetation instance spawner data.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub enum InstanceSpawner {
    Empty(EmptyInstanceSpawner),
    LegacyVegetation(LegacyVegetationInstanceSpawner),
    DynamicSlice(DynamicSliceInstanceSpawner),
}

impl Default for InstanceSpawner {
    fn default() -> Self {
        Self::LegacyVegetation(LegacyVegetationInstanceSpawner::default())
    }
}

impl InstanceSpawner {
    #[must_use]
    pub const fn kind(&self) -> InstanceSpawnerKind {
        match self {
            Self::Empty(_) => InstanceSpawnerKind::Empty,
            Self::LegacyVegetation(_) => InstanceSpawnerKind::LegacyVegetation,
            Self::DynamicSlice(_) => InstanceSpawnerKind::DynamicSlice,
        }
    }

    #[must_use]
    pub fn has_empty_asset_references(&self) -> bool {
        match self {
            Self::Empty(_) => true,
            Self::LegacyVegetation(spawner) => spawner.has_empty_asset_references(),
            Self::DynamicSlice(spawner) => spawner.has_empty_asset_references(),
        }
    }

    #[must_use]
    pub const fn radius(&self) -> f32 {
        match self {
            Self::LegacyVegetation(spawner) => spawner.mesh_radius,
            Self::Empty(_) | Self::DynamicSlice(_) => 0.0,
        }
    }

    #[must_use]
    pub fn scene_asset_path(&self) -> Option<&str> {
        match self {
            Self::Empty(_) => None,
            Self::LegacyVegetation(spawner) => non_empty_path(spawner.mesh_asset_path.as_deref()),
            Self::DynamicSlice(spawner) => non_empty_path(spawner.slice_asset_path.as_deref()),
        }
    }

    #[must_use]
    pub fn scene_asset_variant(&self) -> Option<&str> {
        match self {
            Self::DynamicSlice(spawner) => non_empty_path(spawner.slice_variant.as_deref()),
            Self::Empty(_) | Self::LegacyVegetation(_) => None,
        }
    }
}
