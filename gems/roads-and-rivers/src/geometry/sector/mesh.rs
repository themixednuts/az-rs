use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;

use super::core::SplineGeometrySector;
use crate::geometry::math::mesh_index;

/// Convert 4-point sectors to a Bevy triangle mesh.
pub fn spline_geometry_sectors_to_mesh(
    sectors: impl IntoIterator<Item = SplineGeometrySector>,
) -> Mesh {
    let sectors = sectors.into_iter();
    let capacity = sectors
        .size_hint()
        .1
        .unwrap_or_else(|| sectors.size_hint().0);
    let mut positions = Vec::with_capacity(capacity * 4);
    let mut normals = Vec::with_capacity(capacity * 4);
    let mut uvs = Vec::with_capacity(capacity * 4);
    let mut indices = Vec::with_capacity(capacity * 6);

    for sector in sectors {
        let base = mesh_index(positions.len());
        let normal = sector_normal(&sector);
        for point in sector.points {
            positions.push(point.to_array());
            normals.push(normal.to_array());
        }

        uvs.push([sector.t0.abs(), 0.0]);
        uvs.push([sector.t0.abs(), 1.0]);
        uvs.push([sector.t1.abs(), 0.0]);
        uvs.push([sector.t1.abs(), 1.0]);

        indices.extend_from_slice(&[base, base + 2, base + 3, base, base + 3, base + 1]);
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

#[must_use]
pub fn no_degenerate_triangles(sector: &SplineGeometrySector) -> bool {
    sector.points[0] != sector.points[1]
        && sector.points[0] != sector.points[2]
        && sector.points[1] != sector.points[2]
        && sector.points[2] != sector.points[3]
        && sector.points[1] != sector.points[3]
}

fn sector_normal(sector: &SplineGeometrySector) -> Vec3 {
    (sector.points[2] - sector.points[0])
        .cross(sector.points[3] - sector.points[0])
        .normalize_or_zero()
}
