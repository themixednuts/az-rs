use std::sync::Arc;

use az_nv_cloth::{ClothFabricAsset, ClothMaterial, ClothPaintMaps};
use bevy::prelude::{Quat, Transform, Vec3};

use crate::fabric::{DistanceConstraint, SharedFabric, TetherConstraint};

const DEFAULT_FIXED_STEP: f32 = 1.0 / 60.0;
const MAX_FRAME_DELTA: f32 = 0.25;
const MAX_SUBSTEPS: u32 = 8;
const MIN_DISTANCE: f32 = 1.0e-6;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothParticleTarget {
    pub position: Vec3,
    pub normal: Vec3,
}

impl ClothParticleTarget {
    #[must_use]
    pub fn new(position: Vec3, normal: Vec3) -> Self {
        Self {
            position,
            normal: normal.normalize_or(Vec3::Z),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClothCapsuleCollider {
    pub current: Transform,
    pub previous: Transform,
    pub half_length: f32,
    pub radius: f32,
}

impl ClothCapsuleCollider {
    #[must_use]
    pub fn from_proxy_parameters(
        parameters: bevy::math::Vec4,
        current: Transform,
        previous: Transform,
    ) -> Self {
        Self {
            current,
            previous,
            half_length: parameters.x.max(0.0),
            radius: parameters.w.max(0.0),
        }
    }

    fn segment(self, transform: Transform) -> [Vec3; 2] {
        let axis = transform.rotation * Vec3::X * (self.half_length * transform.scale.x.abs());
        [transform.translation - axis, transform.translation + axis]
    }

    fn radius(self, transform: Transform) -> f32 {
        self.radius * transform.scale.y.abs().max(transform.scale.z.abs())
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ClothSimulationFrame<'a> {
    pub particle_targets: &'a [ClothParticleTarget],
    pub colliders: &'a [ClothCapsuleCollider],
    pub root: Transform,
    pub gravity: Vec3,
    pub local_wind: Vec3,
    pub max_simulation_distance: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClothAdvanceResult {
    Simulated { substeps: u32 },
    WaitingForTargets,
    ResetAfterTeleport,
    RecoveredInvalidState,
}

#[derive(Debug, Clone, Copy)]
struct Particle {
    position: Vec3,
    previous: Vec3,
    inverse_mass: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct CellParticle {
    cell: [i32; 3],
    particle: u32,
}

#[derive(Debug)]
pub struct ClothSolver {
    fabric: Arc<SharedFabric>,
    material: ClothMaterial,
    paint: ClothPaintMaps,
    particles: Box<[Particle]>,
    normals: Box<[Vec3]>,
    render_positions: [Box<[Vec3]>; 2],
    render_buffer: usize,
    self_collision_cells: Vec<CellParticle>,
    acceleration_samples: Box<[Vec3]>,
    acceleration_cursor: usize,
    acceleration_sample_count: usize,
    accumulator: f32,
    fixed_step: f32,
    last_root: Option<Transform>,
}

impl ClothSolver {
    #[must_use]
    pub fn new(
        asset: &ClothFabricAsset,
        fabric: Arc<SharedFabric>,
        material: ClothMaterial,
    ) -> Self {
        let mesh = &asset.fabric().mesh;
        let particles = mesh
            .vertices
            .iter()
            .map(|vertex| Particle {
                position: vertex.position,
                previous: vertex.position,
                inverse_mass: 1.0,
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let positions = mesh
            .vertices
            .iter()
            .map(|vertex| vertex.position)
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let acceleration_samples = acceleration_filter(material);
        Self {
            fabric,
            material,
            paint: mesh.paint.clone(),
            normals: vec![Vec3::Z; particles.len()].into_boxed_slice(),
            render_positions: [positions.clone(), positions],
            render_buffer: 0,
            self_collision_cells: Vec::with_capacity(particles.len()),
            acceleration_samples,
            acceleration_cursor: 0,
            acceleration_sample_count: 0,
            particles,
            accumulator: 0.0,
            fixed_step: DEFAULT_FIXED_STEP,
            last_root: None,
        }
    }

    #[must_use]
    pub fn particle_count(&self) -> usize {
        self.particles.len()
    }

    #[must_use]
    pub fn positions(&self) -> &[Vec3] {
        &self.render_positions[self.render_buffer]
    }

    #[must_use]
    pub fn normals(&self) -> &[Vec3] {
        &self.normals
    }

    #[must_use]
    pub const fn material(&self) -> ClothMaterial {
        self.material
    }

    pub fn set_material(&mut self, material: ClothMaterial) {
        if self.material == material {
            return;
        }
        let filter_len = acceleration_filter_len(material);
        if self.acceleration_samples.len() != filter_len {
            self.acceleration_samples = vec![Vec3::ZERO; filter_len].into_boxed_slice();
            self.acceleration_cursor = 0;
            self.acceleration_sample_count = 0;
        }
        self.material = material;
    }

    pub fn reset_to_targets(&mut self, targets: &[ClothParticleTarget]) -> bool {
        if targets.len() != self.particles.len() {
            return false;
        }
        for (particle, target) in self.particles.iter_mut().zip(targets) {
            particle.position = target.position;
            particle.previous = target.position;
        }
        self.accumulator = 0.0;
        self.acceleration_samples.fill(Vec3::ZERO);
        self.acceleration_cursor = 0;
        self.acceleration_sample_count = 0;
        self.publish_positions();
        self.calculate_normals();
        true
    }

    pub fn advance(
        &mut self,
        frame_delta: f32,
        frame: ClothSimulationFrame<'_>,
    ) -> ClothAdvanceResult {
        if frame.particle_targets.len() != self.particles.len() {
            return ClothAdvanceResult::WaitingForTargets;
        }
        if let Some(previous_root) = self.last_root {
            let translation_delta = frame.root.translation - previous_root.translation;
            if frame.max_simulation_distance > 0.0
                && translation_delta.length_squared()
                    > frame.max_simulation_distance * frame.max_simulation_distance
            {
                self.last_root = Some(frame.root);
                self.reset_to_targets(frame.particle_targets);
                return ClothAdvanceResult::ResetAfterTeleport;
            }
            self.apply_root_inertia(previous_root, frame.root, self.fixed_step);
        } else {
            self.reset_to_targets(frame.particle_targets);
        }
        self.last_root = Some(frame.root);

        if !frame_delta.is_finite() || frame_delta <= 0.0 {
            return ClothAdvanceResult::Simulated { substeps: 0 };
        }
        self.accumulator += frame_delta.min(MAX_FRAME_DELTA);
        let mut substeps = 0;
        while self.accumulator >= self.fixed_step && substeps < MAX_SUBSTEPS {
            self.step(self.fixed_step, frame);
            self.accumulator -= self.fixed_step;
            substeps += 1;
        }
        if substeps == MAX_SUBSTEPS {
            self.accumulator = self.accumulator.min(self.fixed_step);
        }

        if self
            .particles
            .iter()
            .any(|particle| !particle.position.is_finite() || !particle.previous.is_finite())
        {
            self.reset_to_targets(frame.particle_targets);
            return ClothAdvanceResult::RecoveredInvalidState;
        }
        if substeps != 0 {
            self.publish_positions();
            self.calculate_normals();
        }
        ClothAdvanceResult::Simulated { substeps }
    }

    fn step(&mut self, delta: f32, frame: ClothSimulationFrame<'_>) {
        self.integrate(delta, frame.gravity, frame.local_wind);
        let iterations = saturating_u32((self.material.solver_frequency * delta).ceil().max(1.0));
        for _ in 0..iterations {
            self.solve_distance_constraints();
            self.solve_tethers();
            self.solve_motion_constraints(frame.particle_targets);
            self.solve_backstops(frame.particle_targets);
            self.solve_colliders(frame.colliders);
            self.solve_self_collision();
        }
    }

    fn integrate(&mut self, delta: f32, gravity: Vec3, wind: Vec3) {
        let damping = (Vec3::ONE - self.material.damping * delta).clamp(Vec3::ZERO, Vec3::ONE);
        let linear_drag =
            (Vec3::ONE - self.material.linear_drag * delta).clamp(Vec3::ZERO, Vec3::ONE);
        let acceleration = self.filter_acceleration(gravity + wind);
        for particle in &mut self.particles {
            if particle.inverse_mass <= 0.0 {
                continue;
            }
            let velocity = (particle.position - particle.previous) * damping * linear_drag;
            particle.previous = particle.position;
            particle.position += velocity + acceleration * (delta * delta);
        }
    }

    fn apply_root_inertia(&mut self, previous: Transform, current: Transform, delta: f32) {
        let translation = current.rotation.inverse()
            * (current.translation - previous.translation)
            * self.material.linear_inertia;
        let rotation_delta = current.rotation.inverse() * previous.rotation;
        let angular_drag =
            (Vec3::ONE - self.material.angular_drag * delta).clamp(Vec3::ZERO, Vec3::ONE);
        let scaled_axis =
            rotation_delta.to_scaled_axis() * self.material.angular_inertia * angular_drag;
        let rotation = Quat::from_scaled_axis(scaled_axis);
        let angular_velocity = if delta > 0.0 {
            scaled_axis / delta
        } else {
            Vec3::ZERO
        };
        for particle in &mut self.particles {
            let position = rotation * (particle.position - translation);
            let previous_position = rotation * (particle.previous - translation);
            let centrifugal = angular_velocity.cross(angular_velocity.cross(position))
                * self.material.centrifugal_inertia
                * (delta * delta);
            particle.position = position - centrifugal;
            particle.previous = previous_position;
        }
    }

    fn solve_distance_constraints(&mut self) {
        for constraint in self.fabric.constraints.iter().copied() {
            let phase = self.material.phase_configs.for_type(constraint.phase_type);
            let [left, right] = constraint.particles.map(|index| index as usize);
            let Some((left, right)) = get_two_mut(&mut self.particles, left, right) else {
                continue;
            };
            let delta = right.position - left.position;
            let length = delta.length();
            if length <= MIN_DISTANCE || constraint.rest_length <= MIN_DISTANCE {
                continue;
            }
            let ratio = length / constraint.rest_length;
            let outside_limits = ratio < phase.compression_limit || ratio > phase.stretch_limit;
            let stiffness = stiffness_for_step(
                phase.stiffness * constraint.stiffness,
                self.material.stiffness_frequency,
                self.fixed_step,
            ) * if outside_limits {
                phase.stiffness_multiplier
            } else {
                1.0
            };
            solve_pair(
                left,
                right,
                delta / length * (length - constraint.rest_length),
                stiffness,
            );
        }
    }

    fn solve_tethers(&mut self) {
        let scale = self.material.tether_constraints.scale.max(0.0);
        let stiffness = stiffness_for_step(
            self.material.tether_constraints.stiffness,
            self.material.stiffness_frequency,
            self.fixed_step,
        );
        for index in 0..self.fabric.tethers.len() {
            let tether = self.fabric.tethers[index];
            self.solve_tether(tether, scale, stiffness);
        }
    }

    fn solve_tether(&mut self, tether: TetherConstraint, scale: f32, stiffness: f32) {
        let Some((particle, anchor)) = get_two_mut(
            &mut self.particles,
            tether.particle as usize,
            tether.anchor as usize,
        ) else {
            return;
        };
        let delta = particle.position - anchor.position;
        let length = delta.length();
        let maximum = tether.length * scale;
        if length <= maximum || length <= MIN_DISTANCE {
            return;
        }
        let correction = delta / length * (length - maximum) * stiffness;
        let total_mass = particle.inverse_mass + anchor.inverse_mass;
        if total_mass <= 0.0 {
            return;
        }
        particle.position -= correction * (particle.inverse_mass / total_mass);
        anchor.position += correction * (anchor.inverse_mass / total_mass);
    }

    fn solve_motion_constraints(&mut self, targets: &[ClothParticleTarget]) {
        let config = self.material.motion_constraints;
        let stiffness = stiffness_for_step(
            config.stiffness,
            self.material.stiffness_frequency,
            self.fixed_step,
        );
        for (index, (particle, target)) in self.particles.iter_mut().zip(targets).enumerate() {
            let painted = self
                .paint
                .motion_constraint_max_distances
                .get(index)
                .copied()
                .unwrap_or(1.0);
            let radius = (painted * config.max_distance)
                .mul_add(config.scale, config.bias)
                .max(0.0);
            let delta = particle.position - target.position;
            let distance = delta.length();
            if radius <= MIN_DISTANCE {
                particle.position = target.position;
                particle.previous = target.position;
                continue;
            }
            if distance > radius && distance > MIN_DISTANCE {
                particle.position -= delta / distance * (distance - radius) * stiffness;
            }
        }
    }

    fn solve_backstops(&mut self, targets: &[ClothParticleTarget]) {
        let (Some(offsets), Some(radii)) = (
            self.paint.backstop_offsets.as_deref(),
            self.paint.backstop_radii.as_deref(),
        ) else {
            return;
        };
        for (((particle, target), offset), radius) in self
            .particles
            .iter_mut()
            .zip(targets)
            .zip(offsets)
            .zip(radii)
        {
            if *radius <= 0.0 {
                continue;
            }
            let normal = target.normal.normalize_or(Vec3::Z);
            let center = if *offset >= 0.0 {
                target.position - normal * (*radius + *offset)
            } else {
                target.position + normal * (*radius - *offset)
            };
            project_outside_sphere(&mut particle.position, center, *radius);
        }
    }

    fn solve_colliders(&mut self, colliders: &[ClothCapsuleCollider]) {
        for particle in &mut self.particles {
            for collider in colliders
                .iter()
                .copied()
                .filter(|collider| collider.radius > 0.0)
            {
                let current = collider.segment(collider.current);
                if self.material.continuous_collision {
                    let previous = collider.segment(collider.previous);
                    solve_continuous_capsule_collision(
                        particle,
                        previous,
                        current,
                        collider.radius(collider.previous),
                        collider.radius(collider.current),
                    );
                }
                let radius = collider.radius(collider.current);
                project_outside_capsule(&mut particle.position, current[0], current[1], radius);
            }
        }
    }

    fn solve_self_collision(&mut self) {
        let distance = self.material.self_collision.distance;
        let stiffness = stiffness_for_step(
            self.material.self_collision.stiffness,
            self.material.stiffness_frequency,
            self.fixed_step,
        );
        if distance <= MIN_DISTANCE || stiffness <= 0.0 {
            return;
        }
        self.self_collision_cells.clear();
        self.self_collision_cells
            .extend(
                self.particles
                    .iter()
                    .enumerate()
                    .filter_map(|(particle, value)| {
                        u32::try_from(particle).ok().map(|particle| CellParticle {
                            cell: cell_for(value.position, distance),
                            particle,
                        })
                    }),
            );
        self.self_collision_cells.sort_unstable();

        for particle_index in 0..self.particles.len() {
            // Cells above hold `u32` particle ids, so a fabric with more than
            // `u32::MAX` particles has no cell entries past this point either.
            let Ok(particle_id) = u32::try_from(particle_index) else {
                break;
            };
            let cell = cell_for(self.particles[particle_index].position, distance);
            for x in cell[0] - 1..=cell[0] + 1 {
                for y in cell[1] - 1..=cell[1] + 1 {
                    for z in cell[2] - 1..=cell[2] + 1 {
                        let neighbor = [x, y, z];
                        let start = self
                            .self_collision_cells
                            .partition_point(|entry| entry.cell < neighbor);
                        let end = self
                            .self_collision_cells
                            .partition_point(|entry| entry.cell <= neighbor);
                        for entry in &self.self_collision_cells[start..end] {
                            let other_index = entry.particle as usize;
                            if other_index <= particle_index
                                || self
                                    .fabric
                                    .excludes_self_collision(particle_id, entry.particle)
                            {
                                continue;
                            }
                            let Some((left, right)) =
                                get_two_mut(&mut self.particles, particle_index, other_index)
                            else {
                                continue;
                            };
                            let delta = right.position - left.position;
                            let length = delta.length();
                            if length >= distance || length <= MIN_DISTANCE {
                                continue;
                            }
                            solve_pair(
                                left,
                                right,
                                delta / length * (length - distance),
                                stiffness,
                            );
                        }
                    }
                }
            }
        }
    }

    fn calculate_normals(&mut self) {
        self.normals.fill(Vec3::ZERO);
        for triangle in &self.fabric.triangles {
            let [a, b, c] = triangle.map(|index| index as usize);
            let (Some(a_position), Some(b_position), Some(c_position)) = (
                self.particles.get(a).map(|particle| particle.position),
                self.particles.get(b).map(|particle| particle.position),
                self.particles.get(c).map(|particle| particle.position),
            ) else {
                continue;
            };
            let normal = (b_position - a_position).cross(c_position - a_position);
            if let Some(value) = self.normals.get_mut(a) {
                *value += normal;
            }
            if let Some(value) = self.normals.get_mut(b) {
                *value += normal;
            }
            if let Some(value) = self.normals.get_mut(c) {
                *value += normal;
            }
        }
        for normal in &mut self.normals {
            *normal = normal.normalize_or(Vec3::Z);
        }
    }

    fn publish_positions(&mut self) {
        let next = self.render_buffer ^ 1;
        for (destination, particle) in self.render_positions[next].iter_mut().zip(&self.particles) {
            *destination = particle.position;
        }
        self.render_buffer = next;
    }

    fn filter_acceleration(&mut self, acceleration: Vec3) -> Vec3 {
        if self.acceleration_samples.is_empty() {
            return acceleration;
        }
        self.acceleration_samples[self.acceleration_cursor] = acceleration;
        self.acceleration_cursor = (self.acceleration_cursor + 1) % self.acceleration_samples.len();
        self.acceleration_sample_count =
            (self.acceleration_sample_count + 1).min(self.acceleration_samples.len());
        // The filter is at most 1024 samples wide (see
        // `acceleration_filter_len`), so the count is exact in `u16` and `f32`.
        let sample_count =
            f32::from(u16::try_from(self.acceleration_sample_count).unwrap_or(u16::MAX));
        self.acceleration_samples[..self.acceleration_sample_count]
            .iter()
            .copied()
            .sum::<Vec3>()
            / sample_count
    }
}

/// Saturating `f32` -> `u32`.
///
/// `as` is the conversion here, not a shortcut: a float-to-integer `as` cast is
/// defined to saturate (`NaN` and negatives to `0`, out-of-range to
/// [`u32::MAX`]) and std offers no `TryFrom<f32> for u32` to use instead.
#[inline]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn saturating_u32(value: f32) -> u32 {
    value as u32
}

/// Saturating `f32` -> `i32`, for the same reason as [`saturating_u32`].
#[inline]
#[allow(clippy::cast_possible_truncation)]
const fn saturating_i32(value: f32) -> i32 {
    value as i32
}

/// Saturating `f32` -> `usize`, for the same reason as [`saturating_u32`], and
/// `const` so it is usable from [`acceleration_filter_len`].
#[inline]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
const fn saturating_usize(value: f32) -> usize {
    value as usize
}

fn acceleration_filter(material: ClothMaterial) -> Box<[Vec3]> {
    vec![Vec3::ZERO; acceleration_filter_len(material)].into_boxed_slice()
}

const fn acceleration_filter_len(material: ClothMaterial) -> usize {
    saturating_usize(
        material
            .acceleration_filter_width
            .round()
            .clamp(0.0, 1024.0),
    )
}

fn stiffness_for_step(stiffness: f32, frequency: f32, delta: f32) -> f32 {
    let stiffness = stiffness.clamp(0.0, 1.0);
    if stiffness <= 0.0 || frequency <= 0.0 || delta <= 0.0 {
        return 0.0;
    }
    1.0 - (1.0 - stiffness).powf(frequency * delta)
}

fn solve_pair(left: &mut Particle, right: &mut Particle, correction: Vec3, stiffness: f32) {
    let total_mass = left.inverse_mass + right.inverse_mass;
    if total_mass <= 0.0 {
        return;
    }
    let correction = correction * stiffness.clamp(0.0, 1.0);
    left.position += correction * (left.inverse_mass / total_mass);
    right.position -= correction * (right.inverse_mass / total_mass);
}

fn get_two_mut<T>(values: &mut [T], left: usize, right: usize) -> Option<(&mut T, &mut T)> {
    if left == right || left >= values.len() || right >= values.len() {
        return None;
    }
    if left < right {
        let (before, after) = values.split_at_mut(right);
        Some((&mut before[left], &mut after[0]))
    } else {
        let (before, after) = values.split_at_mut(left);
        Some((&mut after[0], &mut before[right]))
    }
}

fn project_outside_sphere(position: &mut Vec3, center: Vec3, radius: f32) {
    let delta = *position - center;
    let distance_squared = delta.length_squared();
    if distance_squared >= radius * radius {
        return;
    }
    *position = if distance_squared > MIN_DISTANCE * MIN_DISTANCE {
        center + delta * (radius / distance_squared.sqrt())
    } else {
        center + Vec3::Z * radius
    };
}

fn project_outside_capsule(position: &mut Vec3, start: Vec3, end: Vec3, radius: f32) {
    let closest = closest_point_on_segment(*position, start, end);
    project_outside_sphere(position, closest, radius);
}

fn solve_continuous_capsule_collision(
    particle: &mut Particle,
    previous_capsule: [Vec3; 2],
    current_capsule: [Vec3; 2],
    previous_radius: f32,
    current_radius: f32,
) {
    let particle_motion = particle.position - particle.previous;
    let capsule_start_motion = current_capsule[0] - previous_capsule[0];
    let capsule_end_motion = current_capsule[1] - previous_capsule[1];
    let speed_bound = particle_motion.length()
        + capsule_start_motion
            .length()
            .max(capsule_end_motion.length())
        + (current_radius - previous_radius).abs();
    if speed_bound <= MIN_DISTANCE {
        return;
    }

    let mut time = 0.0;
    let mut hit = None;
    for _ in 0..32 {
        let sample = moving_capsule_sample(
            particle.previous,
            particle_motion,
            previous_capsule,
            current_capsule,
            previous_radius,
            current_radius,
            time,
        );
        if sample.separation <= MIN_DISTANCE {
            hit = Some(sample);
            break;
        }
        let advance = sample.separation / speed_bound;
        if advance <= 1.0e-5 {
            hit = Some(sample);
            break;
        }
        if time + advance >= 1.0 {
            break;
        }
        time += advance * 0.9;
    }

    let Some(hit) = hit else {
        return;
    };
    let normal = (hit.particle - hit.capsule).normalize_or(Vec3::Z);
    let current_contact = current_capsule[0].lerp(current_capsule[1], hit.segment_parameter);
    let collider_remainder = current_contact - hit.capsule;
    let particle_remainder = particle.position - hit.particle;
    let relative_remainder = particle_remainder - collider_remainder;
    let non_penetrating_remainder =
        relative_remainder - normal * relative_remainder.dot(normal).min(0.0);
    particle.position = hit.particle + collider_remainder + non_penetrating_remainder;
}

#[derive(Debug, Clone, Copy)]
struct MovingCapsuleSample {
    particle: Vec3,
    capsule: Vec3,
    segment_parameter: f32,
    separation: f32,
}

fn moving_capsule_sample(
    particle_start: Vec3,
    particle_motion: Vec3,
    previous_capsule: [Vec3; 2],
    current_capsule: [Vec3; 2],
    previous_radius: f32,
    current_radius: f32,
    time: f32,
) -> MovingCapsuleSample {
    let particle = particle_start + particle_motion * time;
    let capsule_start = previous_capsule[0].lerp(current_capsule[0], time);
    let capsule_end = previous_capsule[1].lerp(current_capsule[1], time);
    let segment_parameter = closest_segment_parameter(particle, capsule_start, capsule_end);
    let capsule = capsule_start.lerp(capsule_end, segment_parameter);
    let radius = (current_radius - previous_radius).mul_add(time, previous_radius);
    MovingCapsuleSample {
        particle,
        capsule,
        segment_parameter,
        separation: particle.distance(capsule) - radius,
    }
}

fn closest_point_on_segment(point: Vec3, start: Vec3, end: Vec3) -> Vec3 {
    start.lerp(end, closest_segment_parameter(point, start, end))
}

fn closest_segment_parameter(point: Vec3, start: Vec3, end: Vec3) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= MIN_DISTANCE * MIN_DISTANCE {
        return 0.0;
    }
    ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0)
}

fn cell_for(position: Vec3, cell_size: f32) -> [i32; 3] {
    let scaled = (position / cell_size).floor();
    [
        saturating_i32(scaled.x),
        saturating_i32(scaled.y),
        saturating_i32(scaled.z),
    ]
}

#[allow(dead_code)]
fn solve_distance_constraint_for_test(
    left: &mut Particle,
    right: &mut Particle,
    constraint: DistanceConstraint,
) {
    let delta = right.position - left.position;
    let length = delta.length();
    if length > MIN_DISTANCE {
        solve_pair(
            left,
            right,
            delta / length * (length - constraint.rest_length),
            constraint.stiffness,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_nv_cloth::FabricPhaseType;

    // `radius` is copied verbatim out of the proxy `Vec4`'s `w`, so the
    // assertion is that it is bit-identical to the constant fed in; an epsilon
    // would stop this test noticing a rescaled radius.
    #[allow(clippy::float_cmp)]
    #[test]
    fn sphere_proxy_has_zero_length_capsule() {
        let collider = ClothCapsuleCollider::from_proxy_parameters(
            bevy::math::Vec4::new(0.0, 0.5, 0.5, 0.25),
            Transform::IDENTITY,
            Transform::IDENTITY,
        );
        assert_eq!(collider.segment(collider.current), [Vec3::ZERO; 2]);
        assert_eq!(collider.radius, 0.25);
    }

    #[test]
    fn distance_constraint_preserves_center_of_mass() {
        let mut left = Particle {
            position: Vec3::ZERO,
            previous: Vec3::ZERO,
            inverse_mass: 1.0,
        };
        let mut right = Particle {
            position: Vec3::X * 2.0,
            previous: Vec3::X * 2.0,
            inverse_mass: 1.0,
        };
        solve_distance_constraint_for_test(
            &mut left,
            &mut right,
            DistanceConstraint {
                particles: [0, 1],
                rest_length: 1.0,
                stiffness: 1.0,
                phase_type: FabricPhaseType::Horizontal,
            },
        );
        assert!((left.position - Vec3::X * 0.5).length() < 1.0e-5);
        assert!((right.position - Vec3::X * 1.5).length() < 1.0e-5);
    }

    #[test]
    fn capsule_projection_uses_x_axis_half_length_and_radius() {
        let mut position = Vec3::new(0.5, 0.1, 0.0);
        project_outside_capsule(&mut position, -Vec3::X, Vec3::X, 0.25);
        assert!((position - Vec3::new(0.5, 0.25, 0.0)).length() < 1.0e-5);
    }

    #[test]
    fn continuous_collision_stops_a_particle_crossing_a_sphere() {
        let mut particle = Particle {
            position: Vec3::X * 2.0,
            previous: -Vec3::X * 2.0,
            inverse_mass: 1.0,
        };
        solve_continuous_capsule_collision(
            &mut particle,
            [Vec3::ZERO; 2],
            [Vec3::ZERO; 2],
            0.5,
            0.5,
        );

        assert!(particle.position.x <= -0.499);
    }
}
