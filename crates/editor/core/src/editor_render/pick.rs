// Expansion drops `cfg(test)`-only names and adds unused ones; it does not compile.
#[allow(clippy::wildcard_imports)]
use super::*;

/// Marker for entities that belong to the editor's in-process scene and are
/// eligible for in-process viewport picking.
///
/// Carries a stable id plus an optional authored reference (`document_id` +
/// `object_id`); when present, a viewport pick routes into the
/// authored-selection → inspector path. Placeholder scene primitives leave the
/// authored reference unset until the project-host → editor-world content
/// bridge spawns real authored entities.
#[derive(Component, Debug, Clone)]
pub struct EditorSceneObject {
    pub id: String,
    pub document_id: Option<String>,
    pub object_id: Option<String>,
}

/// Immutable, thread-safe scene-pick acceleration captured from the latest
/// completed main-world update. Pointer-down hit tests use this snapshot while
/// the production thread is extracting/rendering, so they never queue behind a
/// frame while still resolving against the exact transforms that produced it.
#[derive(Clone, Debug)]
pub struct EditorPickSnapshot {
    ray_origin: Vec3,
    ray_directions: [Vec3; 4],
    objects: Vec<EditorPickProxy>,
}

#[derive(Clone, Debug)]
pub enum EditorPickSnapshotResult {
    Scene(crate::gpu::ViewportPickHit),
    Gizmo,
    Miss,
}

#[derive(Clone, Debug)]
pub(super) struct EditorPickProxy {
    hit: Option<crate::gpu::ViewportPickHit>,
    gizmo: bool,
    local_from_world: Affine3A,
    center: Vec3,
    half_extents: Vec3,
    triangles: Vec<[Vec3; 3]>,
}

impl EditorPickProxy {
    pub(super) const fn new(
        hit: Option<crate::gpu::ViewportPickHit>,
        gizmo: bool,
        local_from_world: Affine3A,
        center: Vec3,
        half_extents: Vec3,
        triangles: Vec<[Vec3; 3]>,
    ) -> Self {
        Self {
            hit,
            gizmo,
            local_from_world,
            center,
            half_extents,
            triangles,
        }
    }
}

impl EditorPickSnapshot {
    pub(super) const fn new(
        ray_origin: Vec3,
        ray_directions: [Vec3; 4],
        objects: Vec<EditorPickProxy>,
    ) -> Self {
        Self {
            ray_origin,
            ray_directions,
            objects,
        }
    }

    #[must_use]
    pub fn pick(&self, coord: Vec2) -> EditorPickSnapshotResult {
        let x = coord.x.clamp(0.0, 1.0);
        let y = coord.y.clamp(0.0, 1.0);
        let top = self.ray_directions[0].lerp(self.ray_directions[1], x);
        let bottom = self.ray_directions[2].lerp(self.ray_directions[3], x);
        let direction = top.lerp(bottom, y).normalize_or_zero();
        let mut nearest = f32::INFINITY;
        let mut picked: Option<&EditorPickProxy> = None;
        for object in &self.objects {
            let origin = object.local_from_world.transform_point3(self.ray_origin);
            let local_direction = object.local_from_world.transform_vector3(direction);
            let minimum = object.center - object.half_extents;
            let maximum = object.center + object.half_extents;
            if ray_aabb_distance(origin, local_direction, minimum, maximum).is_some()
                && let Some(distance) = object
                    .triangles
                    .iter()
                    .filter_map(|triangle| {
                        ray_triangle_distance(origin, local_direction, *triangle)
                    })
                    .reduce(f32::min)
                && distance < nearest
            {
                nearest = distance;
                picked = Some(object);
            }
        }
        match picked {
            Some(object) if object.gizmo => EditorPickSnapshotResult::Gizmo,
            Some(object) => object.hit.clone().map_or(
                EditorPickSnapshotResult::Miss,
                EditorPickSnapshotResult::Scene,
            ),
            None => EditorPickSnapshotResult::Miss,
        }
    }
}

pub(super) fn ray_triangle_distance(
    origin: Vec3,
    direction: Vec3,
    triangle: [Vec3; 3],
) -> Option<f32> {
    let edge_a = triangle[1] - triangle[0];
    let edge_b = triangle[2] - triangle[0];
    let cross = direction.cross(edge_b);
    let determinant = edge_a.dot(cross);
    if determinant.abs() <= f32::EPSILON {
        return None;
    }
    let inverse = determinant.recip();
    let offset = origin - triangle[0];
    let u = offset.dot(cross) * inverse;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = offset.cross(edge_a);
    let v = direction.dot(q) * inverse;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let distance = edge_b.dot(q) * inverse;
    (distance >= 0.0).then_some(distance)
}

pub(super) fn mesh_triangles(mesh: &Mesh) -> Vec<[Vec3; 3]> {
    if !mesh.asset_usage.contains(RenderAssetUsages::MAIN_WORLD)
        || mesh.primitive_topology() != PrimitiveTopology::TriangleList
    {
        return Vec::new();
    }
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
    else {
        return Vec::new();
    };
    let indices = mesh.indices().map_or_else(
        || (0..positions.len()).collect(),
        |indices| indices.iter().collect::<Vec<_>>(),
    );
    indices
        .chunks_exact(3)
        .filter_map(|indices| {
            Some([
                Vec3::from(*positions.get(indices[0])?),
                Vec3::from(*positions.get(indices[1])?),
                Vec3::from(*positions.get(indices[2])?),
            ])
        })
        .collect()
}

pub(super) fn mesh_geometry_counts(mesh: &Mesh) -> Option<(u64, u64)> {
    if !mesh.asset_usage.contains(RenderAssetUsages::MAIN_WORLD) {
        return None;
    }

    let vertex_count = mesh.count_vertices() as u64;
    let triangle_count = if mesh.primitive_topology() == PrimitiveTopology::TriangleList {
        mesh.indices()
            .map_or(vertex_count / 3, |indices| indices.len() as u64 / 3)
    } else {
        0
    };
    Some((vertex_count, triangle_count))
}

pub(super) fn ray_aabb_distance(
    origin: Vec3,
    direction: Vec3,
    minimum: Vec3,
    maximum: Vec3,
) -> Option<f32> {
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let component = direction[axis];
        if component.abs() <= f32::EPSILON {
            if origin[axis] < minimum[axis] || origin[axis] > maximum[axis] {
                return None;
            }
            continue;
        }
        let inverse = component.recip();
        let mut first = (minimum[axis] - origin[axis]) * inverse;
        let mut second = (maximum[axis] - origin[axis]) * inverse;
        if first > second {
            std::mem::swap(&mut first, &mut second);
        }
        near = near.max(first);
        far = far.min(second);
        if near > far {
            return None;
        }
    }
    far.is_finite().then_some(near)
}

impl EditorSceneObject {
    /// A placeholder scene primitive with no authored backing.
    pub(super) fn placeholder(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            document_id: None,
            object_id: None,
        }
    }
}
