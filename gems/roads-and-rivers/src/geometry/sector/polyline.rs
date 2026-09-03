use bevy::prelude::*;

use crate::geometry::math::inverse_lerp;

pub(super) fn vertex_distances(vertices: &[Vec3]) -> Vec<f32> {
    let mut distances = Vec::with_capacity(vertices.len());
    let mut distance = 0.0;
    distances.push(distance);
    for pair in vertices.windows(2) {
        distance += pair[0].distance(pair[1]);
        distances.push(distance);
    }
    distances
}

pub(super) fn points_around_polyline(
    vertices: &[Vec3],
    vertex_distances: &[f32],
    distance: f32,
    width: f32,
) -> (Vec3, Vec3) {
    let (position, tangent) = sample_polyline(vertices, vertex_distances, distance);
    let lateral = tangent.cross(Vec3::Y).normalize_or_zero();
    let lateral = if lateral.length_squared() > f32::EPSILON {
        lateral
    } else {
        Vec3::X
    };
    let half_width = width * 0.5;
    (
        position + lateral * half_width,
        position - lateral * half_width,
    )
}

fn sample_polyline(vertices: &[Vec3], vertex_distances: &[f32], distance: f32) -> (Vec3, Vec3) {
    let clamped = distance.clamp(0.0, vertex_distances.last().copied().unwrap_or(0.0));
    for i in 0..vertices.len() - 1 {
        let d0 = vertex_distances[i];
        let d1 = vertex_distances[i + 1];
        if clamped <= d1 || i == vertices.len() - 2 {
            let start = vertices[i];
            let end = vertices[i + 1];
            let tangent = (end - start).normalize_or_zero();
            let t = inverse_lerp(d0, d1, clamped);
            return (start.lerp(end, t), tangent);
        }
    }

    let last = vertices[vertices.len() - 1];
    (last, Vec3::Z)
}
