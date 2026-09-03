//! Polygon prism shape component data.

use az_asset::AssetId;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use spade::{ConstrainedDelaunayTriangulation, Point2, Triangulation};
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};
use thiserror::Error;
use uuid::{Uuid, uuid};

use super::bounds::aabb_from_min_max;

/// Lumberyard `AZ::PolygonPrism` type ID.
pub const POLYGON_PRISM_TYPE_ID: Uuid = uuid!("F01C8BDD-6F24-4344-8945-521A8750B30B");

/// `PolygonPrismCommon` type ID.
pub const POLYGON_PRISM_COMMON_TYPE_ID: Uuid = uuid!("BDB453DE-8A51-42D0-9237-13A9193BE724");

/// Lumberyard `LmbrCentral::PolygonPrismShapeComponent` type ID.
pub const POLYGON_PRISM_SHAPE_COMPONENT_TYPE_ID: Uuid =
    uuid!("AD882674-1D5D-4E40-B079-449B47D2492C");

/// Largest face accepted by the native `PhysX` polygon-prism cooker.
///
/// A prism with `n` sides has `2n` vertices and `n + 2` faces. `PhysX`'s
/// 255-vertex/face convex-mesh limit therefore caps a face at 127 sides.
pub const MAX_POLYGON_PRISM_EDGES: usize = 127;

const SIMPLE_POLYGON_EPSILON: f32 = 1.0e-3;
const CONVEX_MERGE_ANGLE_EPSILON: f32 = 1.0e-5;

/// A convex 2D decomposition ready to be extruded into solver hulls.
#[derive(Debug, Clone, PartialEq)]
pub struct PolygonPrismDecomposition(Vec<Vec<Vec2>>);

impl AsRef<[Vec<Vec2>]> for PolygonPrismDecomposition {
    fn as_ref(&self) -> &[Vec<Vec2>] {
        &self.0
    }
}

impl IntoIterator for PolygonPrismDecomposition {
    type Item = Vec<Vec2>;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

/// Failure to produce the convex-prism inputs accepted by O3DE's cooker.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PolygonPrismDecompositionError {
    #[error("polygon prism requires at least three vertices, found {0}")]
    TooFewVertices(usize),
    #[error("polygon prism vertex {0} is not finite")]
    NonFiniteVertex(usize),
    #[error("polygon prism height must be finite and non-zero")]
    InvalidHeight,
    #[error("polygon prism is degenerate")]
    Degenerate,
    #[error(
        "polygon prism is not simple; non-adjacent edges {first} and {second} touch or intersect"
    )]
    NotSimple { first: usize, second: usize },
    #[error("polygon prism constrained Delaunay triangulation failed: {0}")]
    Triangulation(String),
    #[error("polygon prism decomposition produced no interior faces")]
    Empty,
}

/// Polygon prism geometry data.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/PolygonPrism.cpp:26`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct PolygonPrism {
    pub height: f32,
    pub vertices: Vec<Vec2>,
}

impl Default for PolygonPrism {
    fn default() -> Self {
        Self {
            height: 1.0,
            vertices: Vec::new(),
        }
    }
}

impl PolygonPrism {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    #[must_use]
    pub const fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    #[must_use]
    pub fn local_bounds(&self) -> Option<Aabb3d> {
        let first = self.vertices.first()?;
        let (mut min, mut max) = (*first, *first);

        for vertex in &self.vertices[1..] {
            min = min.min(*vertex);
            max = max.max(*vertex);
        }

        let min = Vec3::new(min.x, min.y, self.height.min(0.0));
        let max = Vec3::new(max.x, max.y, self.height.max(0.0));
        Some(aabb_from_min_max(min, max))
    }

    /// Decomposes this prism's base along the native O3DE route: constrained
    /// Delaunay triangles followed by smallest-angle-first convex face merging
    /// under the 127-edge `PhysX` limit.
    ///
    /// # Errors
    ///
    /// From the leading validation pass:
    /// [`PolygonPrismDecompositionError::TooFewVertices`] for fewer than three
    /// vertices, [`PolygonPrismDecompositionError::InvalidHeight`] for a
    /// non-finite or zero height,
    /// [`PolygonPrismDecompositionError::NonFiniteVertex`] for a non-finite
    /// vertex, [`PolygonPrismDecompositionError::NotSimple`] when two
    /// non-adjacent edges touch, and
    /// [`PolygonPrismDecompositionError::Degenerate`] for a zero-area base.
    /// From the decomposition itself:
    /// [`PolygonPrismDecompositionError::Triangulation`] if the constrained
    /// Delaunay bulk load fails, and [`PolygonPrismDecompositionError::Empty`]
    /// if no triangle centroid lands inside the polygon or convex merging
    /// leaves no face.
    pub fn decompose(&self) -> Result<PolygonPrismDecomposition, PolygonPrismDecompositionError> {
        validate_prism(self)?;

        if self.vertices.len() <= MAX_POLYGON_PRISM_EDGES && is_convex_ccw(&self.vertices) {
            return Ok(PolygonPrismDecomposition(vec![self.vertices.clone()]));
        }

        let points = self
            .vertices
            .iter()
            .map(|vertex| Point2::new(vertex.x, vertex.y))
            .collect();
        let constraints = (0..self.vertices.len())
            .map(|index| [index, (index + 1) % self.vertices.len()])
            .collect();
        let triangulation =
            ConstrainedDelaunayTriangulation::<Point2<f32>>::bulk_load_cdt(points, constraints)
                .map_err(|error| {
                    PolygonPrismDecompositionError::Triangulation(error.to_string())
                })?;

        let triangles = triangulation
            .inner_faces()
            .filter_map(|face| {
                let vertices = face.vertices();
                let triangle = [
                    vertices[0].fix().index(),
                    vertices[1].fix().index(),
                    vertices[2].fix().index(),
                ];
                let centroid = triangle
                    .iter()
                    .map(|&index| self.vertices[index])
                    .sum::<Vec2>()
                    / 3.0;
                point_in_polygon(centroid, &self.vertices).then_some(triangle)
            })
            .collect::<Vec<_>>();
        if triangles.is_empty() {
            return Err(PolygonPrismDecompositionError::Empty);
        }

        HalfEdgeMesh::from_triangles(&self.vertices, &triangles)
            .convex_merge()
            .into_decomposition()
    }
}

fn validate_prism(prism: &PolygonPrism) -> Result<(), PolygonPrismDecompositionError> {
    if prism.vertices.len() < 3 {
        return Err(PolygonPrismDecompositionError::TooFewVertices(
            prism.vertices.len(),
        ));
    }
    if !prism.height.is_finite() || prism.height.abs() <= f32::EPSILON {
        return Err(PolygonPrismDecompositionError::InvalidHeight);
    }
    if let Some(index) = prism.vertices.iter().position(|vertex| !vertex.is_finite()) {
        return Err(PolygonPrismDecompositionError::NonFiniteVertex(index));
    }
    let count = prism.vertices.len();
    for first in 0..count {
        let end = (first + count - 1) % count;
        let mut second = (first + 2) % count;
        while second != end {
            if segment_distance_squared(
                prism.vertices[first],
                prism.vertices[(first + 1) % count],
                prism.vertices[second],
                prism.vertices[(second + 1) % count],
            ) < SIMPLE_POLYGON_EPSILON * SIMPLE_POLYGON_EPSILON
            {
                return Err(PolygonPrismDecompositionError::NotSimple { first, second });
            }
            second = (second + 1) % count;
        }
    }
    if signed_area(&prism.vertices).abs() <= f32::EPSILON {
        return Err(PolygonPrismDecompositionError::Degenerate);
    }
    Ok(())
}

fn signed_triangle_area(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    0.5 * (a.y - c.y).mul_add(-(b.x - c.x), (a.x - c.x) * (b.y - c.y))
}

fn signed_area(vertices: &[Vec2]) -> f32 {
    vertices
        .iter()
        .zip(vertices.iter().cycle().skip(1))
        .map(|(a, b)| b.x.mul_add(-a.y, a.x * b.y))
        .sum::<f32>()
        * 0.5
}

fn point_segment_distance_squared(point: Vec2, start: Vec2, end: Vec2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared < 1.0e-8 {
        return point.distance_squared(start);
    }
    let projection = (point - start).dot(segment);
    if (0.0..=length_squared).contains(&projection) {
        return (point - start - projection / length_squared * segment).length_squared();
    }
    point
        .distance_squared(start)
        .min(point.distance_squared(end))
}

fn segment_distance_squared(a0: Vec2, a1: Vec2, b0: Vec2, b1: Vec2) -> f32 {
    let area1 = signed_triangle_area(a0, a1, b1);
    let area2 = signed_triangle_area(a0, a1, b0);
    if area1 * area2 < 0.0 {
        let area3 = signed_triangle_area(b0, b1, a0);
        let area4 = area3 + area2 - area1;
        if area3 * area4 < 0.0 {
            return 0.0;
        }
    }
    point_segment_distance_squared(a0, b0, b1)
        .min(point_segment_distance_squared(a1, b0, b1))
        .min(point_segment_distance_squared(b0, a0, a1))
        .min(point_segment_distance_squared(b1, a0, a1))
}

fn is_convex_ccw(vertices: &[Vec2]) -> bool {
    (0..vertices.len()).all(|index| {
        signed_triangle_area(
            vertices[index],
            vertices[(index + 1) % vertices.len()],
            vertices[(index + 2) % vertices.len()],
        ) >= 0.0
    })
}

fn point_in_polygon(point: Vec2, vertices: &[Vec2]) -> bool {
    let mut inside = false;
    for (a, b) in vertices.iter().zip(vertices.iter().cycle().skip(1)) {
        if (a.y > point.y) != (b.y > point.y)
            && point.x < (b.x - a.x) * (point.y - a.y) / (b.y - a.y) + a.x
        {
            inside = !inside;
        }
    }
    inside
}

fn internal_angle(previous: Vec2, vertex: Vec2, next: Vec2) -> f32 {
    let incoming = (previous - vertex).normalize();
    let outgoing = (next - vertex).normalize();
    incoming.dot(outgoing).clamp(-1.0, 1.0).acos()
}

#[derive(Debug, Clone)]
struct Face {
    edge: usize,
    edge_count: usize,
    removed: bool,
}

#[derive(Debug, Clone)]
struct HalfEdge {
    face: usize,
    origin: usize,
    previous: usize,
    next: usize,
    twin: Option<usize>,
    previous_angle: f32,
    next_angle: f32,
    dirty: bool,
}

#[derive(Debug, Clone, Copy)]
struct QueuedInternalEdge {
    edges: [usize; 2],
    min_angle: f32,
    sequence: usize,
}

impl PartialEq for QueuedInternalEdge {
    fn eq(&self, other: &Self) -> bool {
        self.min_angle.to_bits() == other.min_angle.to_bits() && self.sequence == other.sequence
    }
}

impl Eq for QueuedInternalEdge {}

impl PartialOrd for QueuedInternalEdge {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedInternalEdge {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .min_angle
            .total_cmp(&self.min_angle)
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

#[derive(Debug)]
struct HalfEdgeMesh<'a> {
    vertices: &'a [Vec2],
    faces: Vec<Face>,
    edges: Vec<HalfEdge>,
    queue: BinaryHeap<QueuedInternalEdge>,
}

impl<'a> HalfEdgeMesh<'a> {
    fn from_triangles(vertices: &'a [Vec2], triangles: &[[usize; 3]]) -> Self {
        let mut faces = Vec::with_capacity(triangles.len());
        let mut edges = Vec::with_capacity(triangles.len() * 3);
        let mut directed_edges = HashMap::with_capacity(triangles.len() * 3);

        for (face_index, triangle) in triangles.iter().enumerate() {
            let first_edge = edges.len();
            faces.push(Face {
                edge: first_edge,
                edge_count: 3,
                removed: false,
            });
            for edge_index in 0..3 {
                let origin = triangle[edge_index];
                let previous_origin = triangle[(edge_index + 2) % 3];
                let next_origin = triangle[(edge_index + 1) % 3];
                let angle = internal_angle(
                    vertices[previous_origin],
                    vertices[origin],
                    vertices[next_origin],
                );
                edges.push(HalfEdge {
                    face: face_index,
                    origin,
                    previous: first_edge + (edge_index + 2) % 3,
                    next: first_edge + (edge_index + 1) % 3,
                    twin: None,
                    previous_angle: angle,
                    next_angle: 0.0,
                    dirty: false,
                });
                directed_edges.insert((origin, next_origin), first_edge + edge_index);
            }
            for edge_index in 0..3 {
                let next_angle = edges[first_edge + (edge_index + 1) % 3].previous_angle;
                edges[first_edge + edge_index].next_angle = next_angle;
            }
        }

        let mut queue = BinaryHeap::new();
        let mut sequence = 0;
        for edge_index in 0..edges.len() {
            let from = edges[edge_index].origin;
            let to = edges[edges[edge_index].next].origin;
            let Some(&twin) = directed_edges.get(&(to, from)) else {
                continue;
            };
            edges[edge_index].twin = Some(twin);
            if edge_index < twin {
                queue.push(QueuedInternalEdge {
                    edges: [edge_index, twin],
                    min_angle: edge_min_angle(&edges, edge_index, twin),
                    sequence,
                });
                sequence += 1;
            }
        }

        Self {
            vertices,
            faces,
            edges,
            queue,
        }
    }

    fn convex_merge(mut self) -> Self {
        while let Some(mut internal) = self.queue.pop() {
            let [first, second] = internal.edges;
            if self.edges[first].dirty || self.edges[second].dirty {
                internal.min_angle = edge_min_angle(&self.edges, first, second);
                self.edges[first].dirty = false;
                self.edges[second].dirty = false;
                self.queue.push(internal);
                continue;
            }

            let first_edge = &self.edges[first];
            let second_edge = &self.edges[second];
            let new_angle_at_first = first_edge.previous_angle + second_edge.next_angle;
            let new_angle_at_second = first_edge.next_angle + second_edge.previous_angle;
            let new_edge_count = self.faces[first_edge.face].edge_count
                + self.faces[second_edge.face].edge_count
                - 2;
            if new_angle_at_first < core::f32::consts::PI + CONVEX_MERGE_ANGLE_EPSILON
                && new_angle_at_second < core::f32::consts::PI + CONVEX_MERGE_ANGLE_EPSILON
                && new_edge_count <= MAX_POLYGON_PRISM_EDGES
            {
                self.remove_internal_edge(first, second, new_edge_count);
            }
        }
        self
    }

    fn remove_internal_edge(&mut self, first: usize, second: usize, new_edge_count: usize) {
        let face_to_keep = self.edges[first].face;
        let face_to_remove = self.edges[second].face;
        let new_angle_at_first = self.edges[first].previous_angle + self.edges[second].next_angle;
        let new_angle_at_second = self.edges[first].next_angle + self.edges[second].previous_angle;
        self.faces[face_to_keep].edge_count = new_edge_count;
        self.faces[face_to_remove].removed = true;
        if self.faces[face_to_keep].edge == first {
            self.faces[face_to_keep].edge = self.edges[first].next;
        }

        let mut current = second;
        for _ in 0..self.faces[face_to_remove].edge_count {
            self.edges[current].face = face_to_keep;
            current = self.edges[current].next;
        }

        let first_previous = self.edges[first].previous;
        let first_next = self.edges[first].next;
        let second_previous = self.edges[second].previous;
        let second_next = self.edges[second].next;
        for adjacent in [first_previous, first_next, second_previous, second_next] {
            self.edges[adjacent].dirty = true;
        }
        self.edges[first_previous].next = second_next;
        self.edges[first_next].previous = second_previous;
        self.edges[second_previous].next = first_next;
        self.edges[second_next].previous = first_previous;
        self.edges[first_previous].next_angle = new_angle_at_first;
        self.edges[first_next].previous_angle = new_angle_at_second;
        self.edges[second_previous].next_angle = new_angle_at_second;
        self.edges[second_next].previous_angle = new_angle_at_first;
    }

    fn into_decomposition(
        self,
    ) -> Result<PolygonPrismDecomposition, PolygonPrismDecompositionError> {
        let mut output = Vec::new();
        for face in self.faces.into_iter().filter(|face| !face.removed) {
            let mut vertices = Vec::with_capacity(face.edge_count);
            let mut edge = face.edge;
            for _ in 0..face.edge_count {
                vertices.push(self.vertices[self.edges[edge].origin]);
                edge = self.edges[edge].next;
            }
            output.push(vertices);
        }
        if output.is_empty() {
            Err(PolygonPrismDecompositionError::Empty)
        } else {
            Ok(PolygonPrismDecomposition(output))
        }
    }
}

fn edge_min_angle(edges: &[HalfEdge], first: usize, second: usize) -> f32 {
    edges[first]
        .previous_angle
        .min(edges[first].next_angle)
        .min(edges[second].previous_angle)
        .min(edges[second].next_angle)
}

/// Lumberyard polygon prism shape configuration.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/PolygonPrismShape.cpp:129`.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct PolygonPrismCommon {
    pub polygon_prism: PolygonPrism,
    pub reduced_vertices: Vec<Vec2>,
}

impl PolygonPrismCommon {
    #[must_use]
    pub fn local_bounds(&self) -> Option<Aabb3d> {
        self.polygon_prism.local_bounds()
    }
}

/// Runtime polygon prism shape component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/PolygonPrismShapeComponent.cpp:66`.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.PolygonPrismShapeComponent", version = 1)]
pub struct PolygonPrismShapeComponent {
    pub configuration: PolygonPrismCommon,
    #[reflect(ignore)]
    #[serde(default, skip)]
    pub polygon_shape_asset_id: AssetId,
}

impl Default for PolygonPrismShapeComponent {
    fn default() -> Self {
        Self {
            configuration: PolygonPrismCommon::default(),
            polygon_shape_asset_id: AssetId::nil(),
        }
    }
}
