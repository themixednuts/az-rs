use std::collections::{BTreeMap, VecDeque};

use glam::Vec3;

use crate::convert;

#[derive(Debug, Clone, Copy)]
pub struct InterpolatedVertex {
    pub position: Vec3,
    pub source: [u32; 3],
    pub weights: [f32; 3],
}

#[derive(Debug)]
pub struct MeshSlice {
    pub triangles: Vec<[u32; 3]>,
    pub vertices: Vec<InterpolatedVertex>,
    pub removed_triangles: u32,
    pub added_triangles: u32,
    pub removed_islands: u32,
}

#[derive(Debug, Clone, Copy)]
struct SeamVertices {
    positive: u32,
    negative: u32,
}

/// Solver-independent port of the topology-changing part of
/// `CTriMesh::Slice`. The cutter is finite: each source triangle is split only
/// by the overlap of its plane-intersection segment with the cutter triangle.
/// Both copies of every cut edge are retained, producing the same open seam as
/// Cry's paired `newVtx` entries.
pub fn slice_mesh(
    vertices: &[Vec3],
    triangles: &[[u32; 3]],
    cutter: [Vec3; 3],
    minimum_edge_length: f32,
    minimum_island_area_fraction: f32,
) -> Option<MeshSlice> {
    let cutter_normal = (cutter[1] - cutter[0])
        .cross(cutter[2] - cutter[0])
        .normalize_or_zero();
    if cutter_normal == Vec3::ZERO {
        return None;
    }

    let mut state = SliceState {
        vertices,
        cutter,
        cutter_normal,
        minimum_edge_length,
        snap_squared: minimum_edge_length * minimum_edge_length,
        additions: Vec::new(),
        seams: BTreeMap::new(),
    };
    let mut output = Vec::with_capacity(triangles.len() * 2);
    let mut removed_triangles = 0_u32;
    let mut added_triangles = 0_u32;

    for &source in triangles {
        match state.split(source) {
            SplitOutcome::Unchanged => output.push(source),
            SplitOutcome::Split(generated) => {
                removed_triangles += 1;
                added_triangles += convert::u32_from_usize(generated.len());
                output.extend(generated);
            }
            SplitOutcome::Failed => return None,
        }
    }

    if removed_triangles == 0 {
        return None;
    }
    let additions = state.additions;
    let (triangles, removed_islands) =
        remove_small_islands(output, vertices, &additions, minimum_island_area_fraction);
    Some(MeshSlice {
        triangles,
        vertices: additions,
        removed_triangles,
        added_triangles,
        removed_islands,
    })
}

/// What one source triangle contributed to the sliced mesh.
enum SplitOutcome {
    /// The cutter leaves the triangle intact; the source is kept as authored.
    Unchanged,
    /// The triangle is replaced by these fragments.
    Split(Vec<[u32; 3]>),
    /// Local retriangulation failed and the whole slice is abandoned.
    Failed,
}

/// Cutter and accumulated seam state shared by every source triangle of one
/// slice.
struct SliceState<'a> {
    vertices: &'a [Vec3],
    cutter: [Vec3; 3],
    cutter_normal: Vec3,
    minimum_edge_length: f32,
    snap_squared: f32,
    additions: Vec<InterpolatedVertex>,
    seams: BTreeMap<[i64; 3], SeamVertices>,
}

impl SliceState<'_> {
    fn split(&mut self, source: [u32; 3]) -> SplitOutcome {
        let source_points = source.map(|index| self.vertices[index as usize]);
        let source_normal =
            (source_points[1] - source_points[0]).cross(source_points[2] - source_points[0]);
        let Some([mut start, mut end]) = triangle_intersection_segment(
            source_points,
            source_normal,
            self.cutter,
            self.cutter_normal,
        ) else {
            return SplitOutcome::Unchanged;
        };

        start = snap_to_triangle_vertex(start, source_points, self.snap_squared);
        end = snap_to_triangle_vertex(end, source_points, self.snap_squared);
        if start.distance_squared(end) <= f32::EPSILON
            || segment_is_triangle_boundary(start, end, source_points, self.snap_squared)
        {
            return SplitOutcome::Unchanged;
        }

        let mut local_points = source_points.to_vec();
        let mut local_triangles = vec![[0_usize, 1, 2]];
        let Some(start_local) = insert_point(
            &mut local_points,
            &mut local_triangles,
            start,
            source_normal,
        ) else {
            return SplitOutcome::Failed;
        };
        let Some(end_local) =
            insert_point(&mut local_points, &mut local_triangles, end, source_normal)
        else {
            return SplitOutcome::Failed;
        };
        if start_local == end_local {
            return SplitOutcome::Unchanged;
        }

        let start_weights = barycentric(start, source_points);
        let end_weights = barycentric(end, source_points);
        let start_seam = self.seam(
            start,
            snapped_source(start, source, source_points, self.snap_squared),
            source,
            start_weights,
        );
        let end_seam = self.seam(
            end,
            snapped_source(end, source, source_points, self.snap_squared),
            source,
            end_weights,
        );

        let generated = self.retriangulate(
            &local_points,
            local_triangles,
            SeamSpan {
                start,
                end,
                start_local,
                end_local,
                start_seam,
                end_seam,
            },
            source,
            source_normal,
        );
        if generated.len() < 2 {
            return SplitOutcome::Unchanged;
        }
        SplitOutcome::Split(generated)
    }

    /// Rewrites the locally retriangulated fragments onto mesh-wide indices,
    /// sending each fragment to the seam copy that lies on its own side of the
    /// cut.
    fn retriangulate(
        &self,
        local_points: &[Vec3],
        local_triangles: Vec<[usize; 3]>,
        span: SeamSpan,
        source: [u32; 3],
        source_normal: Vec3,
    ) -> Vec<[u32; 3]> {
        let mut generated = Vec::with_capacity(local_triangles.len());
        for triangle in local_triangles {
            let centroid = triangle
                .into_iter()
                .map(|index| local_points[index])
                .sum::<Vec3>()
                / 3.0;
            let positive =
                source_normal.dot((span.end - span.start).cross(centroid - span.start)) >= 0.0;
            let mapped = triangle.map(|local| span.index(local, positive, source));
            if mapped[0] == mapped[1] || mapped[1] == mapped[2] || mapped[2] == mapped[0] {
                continue;
            }
            generated.push(orient(
                mapped,
                source_normal,
                self.vertices,
                &self.additions,
            ));
        }
        generated
    }

    /// Allocates, or reuses across triangles, the paired seam vertices for one
    /// cut point.
    fn seam(
        &mut self,
        position: Vec3,
        snapped: Option<u32>,
        source: [u32; 3],
        weights: [f32; 3],
    ) -> SeamVertices {
        let quantization = 4096.0 / self.minimum_edge_length.max(1.0e-6);
        let key = position
            .to_array()
            .map(|value| convert::i64_from_f32(value * quantization));
        if let Some(vertices) = self.seams.get(&key) {
            return *vertices;
        }
        let source_vertex_count = self.vertices.len();
        let additions = &mut self.additions;
        let mut allocate = || {
            let index = convert::u32_from_usize(source_vertex_count + additions.len());
            additions.push(InterpolatedVertex {
                position,
                source,
                weights,
            });
            index
        };
        let vertices = SeamVertices {
            positive: snapped.unwrap_or_else(&mut allocate),
            negative: allocate(),
        };
        self.seams.insert(key, vertices);
        vertices
    }
}

/// The cut segment of one source triangle, in both local and seam indices.
#[derive(Debug, Clone, Copy)]
struct SeamSpan {
    start: Vec3,
    end: Vec3,
    start_local: usize,
    end_local: usize,
    start_seam: SeamVertices,
    end_seam: SeamVertices,
}

impl SeamSpan {
    /// Maps one local vertex of a fragment onto its mesh-wide index, choosing
    /// the seam copy on the fragment's side of the cut.
    const fn index(self, local: usize, positive: bool, source: [u32; 3]) -> u32 {
        let seam = if local == self.start_local {
            self.start_seam
        } else if local == self.end_local {
            self.end_seam
        } else {
            return source[local];
        };
        if positive {
            seam.positive
        } else {
            seam.negative
        }
    }
}

fn triangle_intersection_segment(
    left: [Vec3; 3],
    left_normal: Vec3,
    right: [Vec3; 3],
    right_normal: Vec3,
) -> Option<[Vec3; 2]> {
    let direction = left_normal.cross(right_normal);
    let direction_squared = direction.length_squared();
    if direction_squared <= 1.0e-12 {
        return None;
    }
    let left_segment = triangle_plane_segment(left, right[0], right_normal)?;
    let right_segment = triangle_plane_segment(right, left[0], left_normal)?;
    let axis = direction / direction_squared.sqrt();
    let origin = left_segment[0];
    let interval = |segment: [Vec3; 2]| {
        let a = (segment[0] - origin).dot(axis);
        let b = (segment[1] - origin).dot(axis);
        [a.min(b), a.max(b)]
    };
    let left_interval = interval(left_segment);
    let right_interval = interval(right_segment);
    let start = left_interval[0].max(right_interval[0]);
    let end = left_interval[1].min(right_interval[1]);
    (end - start > 1.0e-6).then_some([origin + axis * start, origin + axis * end])
}

fn triangle_plane_segment(
    triangle: [Vec3; 3],
    plane_point: Vec3,
    plane_normal: Vec3,
) -> Option<[Vec3; 2]> {
    let distances = triangle.map(|point| plane_normal.dot(point - plane_point));
    if distances.iter().all(|distance| *distance > 1.0e-6)
        || distances.iter().all(|distance| *distance < -1.0e-6)
    {
        return None;
    }
    let mut intersections = Vec::<Vec3>::with_capacity(3);
    for edge in 0..3 {
        let next = (edge + 1) % 3;
        let a = triangle[edge];
        let b = triangle[next];
        let da = distances[edge];
        let db = distances[next];
        if da.abs() <= 1.0e-6 {
            push_distinct(&mut intersections, a);
        }
        if da * db < -1.0e-12 {
            push_distinct(&mut intersections, a + (b - a) * (da / (da - db)));
        }
    }
    if intersections.len() != 2 {
        return None;
    }
    Some([intersections[0], intersections[1]])
}

fn push_distinct(points: &mut Vec<Vec3>, point: Vec3) {
    if points
        .iter()
        .all(|existing| existing.distance_squared(point) > 1.0e-12)
    {
        points.push(point);
    }
}

fn insert_point(
    points: &mut Vec<Vec3>,
    triangles: &mut Vec<[usize; 3]>,
    point: Vec3,
    normal: Vec3,
) -> Option<usize> {
    if let Some(index) = points
        .iter()
        .position(|candidate| candidate.distance_squared(point) <= 1.0e-12)
    {
        return Some(index);
    }
    let point_index = points.len();
    points.push(point);

    let mut split_edge = None;
    'outer: for triangle in triangles.iter() {
        for edge in 0..3 {
            let a = triangle[edge];
            let b = triangle[(edge + 1) % 3];
            if point_on_segment(point, points[a], points[b]) {
                split_edge = Some((a.min(b), a.max(b)));
                break 'outer;
            }
        }
    }
    if let Some(edge) = split_edge {
        let mut replacement = Vec::with_capacity(triangles.len() + 2);
        let mut split = false;
        for triangle in triangles.drain(..) {
            let Some(opposite) = triangle
                .iter()
                .copied()
                .find(|index| *index != edge.0 && *index != edge.1)
                .filter(|_| triangle.contains(&edge.0) && triangle.contains(&edge.1))
            else {
                replacement.push(triangle);
                continue;
            };
            replacement.push(orient_local(
                [edge.0, point_index, opposite],
                normal,
                points,
            ));
            replacement.push(orient_local(
                [point_index, edge.1, opposite],
                normal,
                points,
            ));
            split = true;
        }
        *triangles = replacement;
        return split.then_some(point_index);
    }

    let triangle_index = triangles.iter().position(|triangle| {
        point_in_triangle(point, triangle.map(|index| points[index]), normal)
    })?;
    let [a, b, c] = triangles.swap_remove(triangle_index);
    triangles.push(orient_local([a, b, point_index], normal, points));
    triangles.push(orient_local([b, c, point_index], normal, points));
    triangles.push(orient_local([c, a, point_index], normal, points));
    Some(point_index)
}

fn point_on_segment(point: Vec3, a: Vec3, b: Vec3) -> bool {
    let edge = b - a;
    let length_squared = edge.length_squared();
    if length_squared <= 1.0e-12 {
        return false;
    }
    let t = (point - a).dot(edge) / length_squared;
    (-1.0e-5..=1.0 + 1.0e-5).contains(&t)
        && point.distance_squared(a + edge * t.clamp(0.0, 1.0)) <= 1.0e-10
}

fn point_in_triangle(point: Vec3, triangle: [Vec3; 3], normal: Vec3) -> bool {
    (0..3).all(|edge| {
        let a = triangle[edge];
        let b = triangle[(edge + 1) % 3];
        normal.dot((b - a).cross(point - a)) >= -1.0e-5
    })
}

fn orient_local(mut triangle: [usize; 3], normal: Vec3, points: &[Vec3]) -> [usize; 3] {
    if (points[triangle[1]] - points[triangle[0]])
        .cross(points[triangle[2]] - points[triangle[0]])
        .dot(normal)
        < 0.0
    {
        triangle.swap(1, 2);
    }
    triangle
}

fn snap_to_triangle_vertex(point: Vec3, triangle: [Vec3; 3], snap_squared: f32) -> Vec3 {
    triangle
        .into_iter()
        .min_by(|left, right| {
            left.distance_squared(point)
                .total_cmp(&right.distance_squared(point))
        })
        .filter(|vertex| vertex.distance_squared(point) < snap_squared)
        .unwrap_or(point)
}

fn snapped_source(
    point: Vec3,
    source: [u32; 3],
    triangle: [Vec3; 3],
    snap_squared: f32,
) -> Option<u32> {
    triangle
        .into_iter()
        .enumerate()
        .find(|(_, vertex)| vertex.distance_squared(point) <= snap_squared.min(1.0e-10))
        .map(|(index, _)| source[index])
}

fn segment_is_triangle_boundary(
    start: Vec3,
    end: Vec3,
    triangle: [Vec3; 3],
    tolerance_squared: f32,
) -> bool {
    (0..3).any(|edge| {
        let a = triangle[edge];
        let b = triangle[(edge + 1) % 3];
        distance_to_segment_squared(start, a, b) <= tolerance_squared * 1.0e-4
            && distance_to_segment_squared(end, a, b) <= tolerance_squared * 1.0e-4
    })
}

fn distance_to_segment_squared(point: Vec3, a: Vec3, b: Vec3) -> f32 {
    let edge = b - a;
    let denominator = edge.length_squared();
    if denominator <= f32::EPSILON {
        return point.distance_squared(a);
    }
    let fraction = ((point - a).dot(edge) / denominator).clamp(0.0, 1.0);
    point.distance_squared(a + edge * fraction)
}

fn barycentric(point: Vec3, [origin, second, third]: [Vec3; 3]) -> [f32; 3] {
    let edge_second = second - origin;
    let edge_third = third - origin;
    let to_point = point - origin;
    let second_second = edge_second.dot(edge_second);
    let second_third = edge_second.dot(edge_third);
    let third_third = edge_third.dot(edge_third);
    let point_second = to_point.dot(edge_second);
    let point_third = to_point.dot(edge_third);
    let denominator = second_third.mul_add(-second_third, second_second * third_third);
    if denominator.abs() <= 1.0e-12 {
        return [1.0, 0.0, 0.0];
    }
    let beta = second_third.mul_add(-point_third, third_third * point_second) / denominator;
    let gamma = second_third.mul_add(-point_second, second_second * point_third) / denominator;
    [1.0 - beta - gamma, beta, gamma]
}

fn orient(
    mut triangle: [u32; 3],
    normal: Vec3,
    vertices: &[Vec3],
    additions: &[InterpolatedVertex],
) -> [u32; 3] {
    let point = |index: u32| {
        vertices
            .get(index as usize)
            .copied()
            .unwrap_or_else(|| additions[index as usize - vertices.len()].position)
    };
    if (point(triangle[1]) - point(triangle[0]))
        .cross(point(triangle[2]) - point(triangle[0]))
        .dot(normal)
        < 0.0
    {
        triangle.swap(1, 2);
    }
    triangle
}

fn remove_small_islands(
    triangles: Vec<[u32; 3]>,
    vertices: &[Vec3],
    additions: &[InterpolatedVertex],
    minimum_fraction: f32,
) -> (Vec<[u32; 3]>, u32) {
    let mut edges = BTreeMap::<(u32, u32), Vec<usize>>::new();
    for (triangle_index, triangle) in triangles.iter().enumerate() {
        for edge in 0..3 {
            let a = triangle[edge];
            let b = triangle[(edge + 1) % 3];
            edges
                .entry((a.min(b), a.max(b)))
                .or_default()
                .push(triangle_index);
        }
    }
    let mut adjacency = vec![Vec::new(); triangles.len()];
    for incident in edges.values() {
        for &left in incident {
            for &right in incident {
                if left != right {
                    adjacency[left].push(right);
                }
            }
        }
    }
    let point = |index: u32| {
        vertices
            .get(index as usize)
            .copied()
            .unwrap_or_else(|| additions[index as usize - vertices.len()].position)
    };
    let mut islands = Vec::<Vec<usize>>::new();
    let mut seen = vec![false; triangles.len()];
    for start in 0..triangles.len() {
        if seen[start] {
            continue;
        }
        let mut queue = VecDeque::from([start]);
        let mut island = Vec::new();
        seen[start] = true;
        while let Some(triangle) = queue.pop_front() {
            island.push(triangle);
            for &next in &adjacency[triangle] {
                if !seen[next] {
                    seen[next] = true;
                    queue.push_back(next);
                }
            }
        }
        islands.push(island);
    }
    let areas: Vec<_> = islands
        .iter()
        .map(|island| {
            island
                .iter()
                .map(|&index| {
                    let [a, b, c] = triangles[index].map(point);
                    (b - a).cross(c - a).length()
                })
                .sum::<f32>()
        })
        .collect();
    let total = areas.iter().sum::<f32>();
    let mut remove = vec![false; triangles.len()];
    let mut removed_islands = 0;
    for (island, area) in islands.into_iter().zip(areas) {
        if area < total * minimum_fraction {
            removed_islands += 1;
            for triangle in island {
                remove[triangle] = true;
            }
        }
    }
    (
        triangles
            .into_iter()
            .enumerate()
            .filter_map(|(index, triangle)| (!remove[index]).then_some(triangle))
            .collect(),
        removed_islands,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finite_cutter_opens_a_shared_seam_and_preserves_area() {
        let vertices = [
            Vec3::new(-1.0, -1.0, 0.0),
            Vec3::new(1.0, -1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
        ];
        let triangles = [[0, 1, 2], [0, 2, 3]];
        let result = slice_mesh(
            &vertices,
            &triangles,
            [
                Vec3::new(0.0, -2.0, -1.0),
                Vec3::new(0.0, 2.0, -1.0),
                Vec3::new(0.0, 0.0, 1.0),
            ],
            0.01,
            0.05,
        )
        .expect("cutter intersects the quad");
        assert_eq!(result.removed_triangles, 2);
        assert!(result.vertices.len() >= 4);
        assert!(result.triangles.len() >= 4);
        assert_eq!(result.removed_islands, 0);
    }

    #[test]
    fn cutter_outside_mesh_is_not_an_infinite_plane_cut() {
        let vertices = [Vec3::ZERO, Vec3::X, Vec3::Y];
        let result = slice_mesh(
            &vertices,
            &[[0, 1, 2]],
            [
                Vec3::new(2.0, 2.0, -1.0),
                Vec3::new(2.0, 3.0, 1.0),
                Vec3::new(3.0, 2.0, 1.0),
            ],
            0.01,
            0.05,
        );
        assert!(result.is_none());
    }
}
