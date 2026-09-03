//! Shared spline strip geometry for roads and rivers.

#[allow(clippy::module_inception)]
mod geometry;
mod math;
mod sector;
mod type_ids;
mod width;

use az_gem_lmbr_central::EngineSpec;
use bevy::prelude::*;

pub use geometry::SplineGeometry;
pub use sector::no_degenerate_triangles;
pub use sector::{
    SplineGeometrySector, build_spline_geometry_sectors, spline_geometry_sectors_to_mesh,
};
pub use type_ids::*;
pub use width::{DistanceWidth, RoadWidthInterpolator, SplineGeometryWidthModifier};

pub fn register_geometry_components(app: &mut App) {
    app.register_type::<EngineSpec>()
        .register_type::<Vec<f32>>()
        .register_type::<Vec<DistanceWidth>>()
        .register_type::<Vec<SplineGeometrySector>>()
        .register_type::<DistanceWidth>()
        .register_type::<RoadWidthInterpolator>()
        .register_type::<SplineGeometryWidthModifier>()
        .register_type::<SplineGeometry>()
        .register_type::<SplineGeometrySector>();
}

#[cfg(test)]
use az_gem_lmbr_central::SplineData;

#[cfg(test)]
mod tests {
    use super::*;
    // The keyframes are 0.0 and 10.0; clamped lookups return those literals
    // unchanged and the midpoint eases to exactly 5.0, so exact equality is
    // what pins the interpolator.
    #[allow(clippy::float_cmp)]
    #[test]
    fn width_interpolator_sorts_and_eases_values() {
        let mut interp = RoadWidthInterpolator::default();
        interp.insert_distance_width_key_frame(10.0, 10.0);
        interp.insert_distance_width_key_frame(0.0, 0.0);

        assert_eq!(interp.width(-1.0), 0.0);
        assert_eq!(interp.width(11.0), 10.0);
        assert_eq!(interp.width(5.0), 5.0);
        assert_eq!(interp.maximum_width(), 10.0);
    }

    // The first sector starts at distance 0, so its `t0` is exactly 0.0.
    #[allow(clippy::float_cmp)]
    #[test]
    fn polyline_geometry_builds_expected_sector_count() {
        let geometry = SplineGeometry {
            segment_length: 2.0,
            tile_length: 10.0,
            ..default()
        };
        let spline = SplineData {
            vertices: vec![Vec3::ZERO, Vec3::Z * 10.0],
            closed: false,
        };

        let sectors = build_spline_geometry_sectors(&geometry, &spline);

        assert_eq!(sectors.len(), 5);
        assert_eq!(sectors[0].t0, 0.0);
        assert!(sectors.last().is_some_and(|sector| sector.t1 < 0.0));
    }
}
