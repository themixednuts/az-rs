use az_gem_lmbr_central::SplineData;
use bevy::prelude::*;

use super::core::SplineGeometrySector;
use super::polyline::{points_around_polyline, vertex_distances};
use crate::geometry::SplineGeometry;
use crate::geometry::math::{ExactF32, ceil_count};

/// Build strip sectors from spline centerline data.
#[must_use]
pub fn build_spline_geometry_sectors(
    geometry: &SplineGeometry,
    spline: &SplineData,
) -> Vec<SplineGeometrySector> {
    let vertices = &spline.vertices;
    if vertices.len() < 2 {
        return Vec::new();
    }

    let vertex_distances = vertex_distances(vertices);
    let Some(total_length) = vertex_distances.last().copied() else {
        return Vec::new();
    };
    if total_length <= f32::EPSILON {
        return Vec::new();
    }

    let chunks_count = ceil_count(total_length / geometry.segment_length.max(f32::EPSILON));
    if chunks_count == 0 {
        return Vec::new();
    }
    let real_step_size = total_length / chunks_count.exact_f32();

    let mut boundaries = Vec::with_capacity(chunks_count + 1);
    for i in 0..=chunks_count {
        let distance = (i.exact_f32() * real_step_size).min(total_length);
        let width = geometry.width.width_at(distance, &vertex_distances);
        let (left, right) = points_around_polyline(vertices, &vertex_distances, distance, width);
        boundaries.push((distance / geometry.tile_length, left, right));
    }

    let mut sectors = Vec::with_capacity(chunks_count);
    for pair in boundaries.windows(2) {
        let (t0, left0, right0) = pair[0];
        let (t1, left1, right1) = pair[1];
        sectors.push(SplineGeometrySector {
            points: [left0, right0, left1, right1],
            t0,
            t1,
        });
    }

    if let Some(last) = sectors.last_mut() {
        last.t1 *= -1.0;
    }

    sectors
}
