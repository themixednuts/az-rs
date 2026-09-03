use az_prefab::{Prefab, ReflectPrefab};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::geometry::{SplineGeometry, spline_geometry_sectors_to_mesh};

/// Default material asset for roads.
pub const DEFAULT_ROAD_MATERIAL: &str = "materials/road/defaultroad.mtl";

/// Road component represented as Bevy ECS data.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.roads_and_rivers.RoadComponent", version = 1)]
pub struct RoadComponent {
    pub geometry: SplineGeometry,
    /// Engine material asset path.
    pub material_path: String,
    pub ignore_terrain_holes: bool,
    pub allow_road_follow: bool,
    pub add_overlap_between_sectors: bool,
}

impl Default for RoadComponent {
    fn default() -> Self {
        Self {
            geometry: SplineGeometry::default(),
            material_path: DEFAULT_ROAD_MATERIAL.to_string(),
            ignore_terrain_holes: false,
            allow_road_follow: false,
            add_overlap_between_sectors: false,
        }
    }
}

impl RoadComponent {
    #[must_use]
    pub fn mesh(&self, spline: &az_gem_lmbr_central::SplineData) -> Mesh {
        spline_geometry_sectors_to_mesh(self.geometry.sectors(spline))
    }
}
