use bevy::mesh::Mesh;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::geometry::{SplineGeometry, no_degenerate_triangles, spline_geometry_sectors_to_mesh};

/// Default material asset for rivers.
pub const DEFAULT_RIVER_MATERIAL: &str = "materials/river/defaultriver.mtl";

/// River component represented as Bevy ECS data.
// The flags are a 1:1 port of the Lumberyard river component's fields and are
// serialized and reflected under those names; folding them into a bitflags or
// enum type would rename the wire shape.
#[allow(clippy::struct_excessive_bools)]
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct RiverComponent {
    pub geometry: SplineGeometry,
    pub material_path: String,
    pub water_volume_depth: f32,
    pub tile_width: f32,
    pub water_cap_fog_at_volume_depth: bool,
    pub water_fog_density: f32,
    pub fog_color: Color,
    pub fog_color_affected_by_sun: bool,
    pub water_fog_shadowing: f32,
    pub water_caustics: bool,
    pub water_caustic_intensity: f32,
    pub water_caustic_height: f32,
    pub water_caustic_tiling: f32,
    pub physicalize: bool,
    pub water_stream_speed: f32,
}

impl Default for RiverComponent {
    fn default() -> Self {
        Self {
            geometry: SplineGeometry::default(),
            material_path: DEFAULT_RIVER_MATERIAL.to_string(),
            water_volume_depth: 10.0,
            tile_width: 1.0,
            water_cap_fog_at_volume_depth: false,
            water_fog_density: 0.5,
            fog_color: Color::BLACK,
            fog_color_affected_by_sun: true,
            water_fog_shadowing: 0.5,
            water_caustics: true,
            water_caustic_intensity: 20.0,
            water_caustic_height: 0.0,
            water_caustic_tiling: 40.0,
            physicalize: false,
            water_stream_speed: 0.0,
        }
    }
}

impl RiverComponent {
    pub fn mesh(&self, spline: &az_gem_lmbr_central::SplineData) -> Mesh {
        spline_geometry_sectors_to_mesh(
            self.geometry
                .sectors(spline)
                .into_iter()
                .filter(no_degenerate_triangles),
        )
    }
}
