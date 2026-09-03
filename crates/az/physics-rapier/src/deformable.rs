use std::collections::{BTreeMap, BTreeSet, VecDeque};

use az_physics::{
    DeformableTargetVertices, ImpulseAction, LinkedSoftBodyAerodynamics,
    LinkedSoftBodyCollisionFeature, LinkedSoftBodyConfiguration, LinkedSoftBodyMediumVelocityMode,
    LinkedSoftBodyStatusRef, PhysicsBodyHandle, PhysicsError, PhysicsPose, RopeAttachment,
    RopeBodyConfiguration, RopeFlags, RopeStatus, RopeTargetPoseMode, RopeVolumetricPressure,
    SoftBodyAttachment, SoftBodyAttachmentUpdate, SoftBodyConfiguration, SoftBodyFlags,
    SoftBodyImpulse, SoftBodyPressure, SoftBodySlice, SoftBodySliceResult, SoftBodyStatus,
};
use glam::{Mat3, Quat, Vec3, Vec4};
use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};
use smallvec::SmallVec;

use crate::convert::{self, f32_from_u32, f32_from_usize, u32_from_usize};

#[derive(Debug, Clone, Copy)]
pub struct AttachmentFrame {
    pub position: Vec3,
    pub body_position: Vec3,
    pub center: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
}

#[derive(Debug, Clone, Copy)]
struct LinkedSoftBodyLink {
    vertices: [usize; 2],
    rest_length_squared: f32,
}

#[derive(Debug, Clone)]
struct LinkedSoftBodyCluster {
    vertices: Box<[usize]>,
    linear_impulse_scale: f32,
}

#[derive(Debug, Clone, Copy)]
struct LinkedSoftBodyClusterFrame {
    center: Vec3,
    previous_center: Vec3,
    inverse_inertia: Mat3,
    linear_velocity: Vec3,
    angular_velocity: Vec3,
    radius: f32,
}

/// Runtime state for `RockNRoll`'s linked triangular deformable.
///
/// This remains separate from [`SoftBodyState`], which is the Cry
/// `CSoftEntity` implementation. The state ordering follows the `RockNRoll`
/// island path: external forces/prediction, constraint projection,
/// contacts, then velocity/status refresh.
#[derive(Debug, Clone)]
pub struct LinkedSoftBodyState {
    pub configuration: LinkedSoftBodyConfiguration,
    pub query_collider: ColliderHandle,
    pub vertices: Vec<Vec3>,
    pub previous_vertices: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    vertex_areas: Vec<f32>,
    target_host: PhysicsPose,
    target_vertices: Vec<Vec3>,
    target_normals: Vec<Vec3>,
    target_active: bool,
    pose_frame: PhysicsPose,
    medium_velocity: Vec3,
    previous_medium_velocity: Vec3,
    target_medium_velocity: Vec3,
    medium_velocity_elapsed: f32,
    medium_velocity_duration: f32,
    medium_velocity_random_state: u64,
    pub awake: bool,
    pub entity_pose: PhysicsPose,
    inverse_masses: Vec<f32>,
    links: Vec<LinkedSoftBodyLink>,
    secondary_links: Vec<LinkedSoftBodyLink>,
    clusters: Vec<LinkedSoftBodyCluster>,
    pending_force: Vec3,
    pending_torque: Vec3,
    reactions: SmallVec<[DeformableReaction; 16]>,
}

impl LinkedSoftBodyState {
    pub(crate) fn new(
        configuration: LinkedSoftBodyConfiguration,
        pose: PhysicsPose,
        query_collider: ColliderHandle,
    ) -> Self {
        let vertices = configuration
            .vertices
            .iter()
            .map(|vertex| pose.transform_point(*vertex))
            .collect::<Vec<_>>();
        let velocities = if configuration.velocities.is_empty() {
            vec![Vec3::ZERO; vertices.len()]
        } else {
            configuration
                .velocities
                .iter()
                .map(|velocity| pose.rotation * *velocity)
                .collect()
        };
        let links = configuration
            .links
            .iter()
            .map(|link| {
                let vertices = [link[0] as usize, link[1] as usize];
                LinkedSoftBodyLink {
                    vertices,
                    rest_length_squared: configuration.vertices[vertices[0]]
                        .distance_squared(configuration.vertices[vertices[1]]),
                }
            })
            .collect();
        let secondary_links = configuration
            .secondary_links
            .iter()
            .map(|link| {
                let vertices = [link[0] as usize, link[1] as usize];
                LinkedSoftBodyLink {
                    vertices,
                    rest_length_squared: configuration.vertices[vertices[0]]
                        .distance_squared(configuration.vertices[vertices[1]]),
                }
            })
            .collect();
        let clusters = configuration
            .clusters
            .iter()
            .map(|cluster| LinkedSoftBodyCluster {
                vertices: cluster
                    .vertices
                    .iter()
                    .map(|&vertex| usize::from(vertex))
                    .collect(),
                linear_impulse_scale: cluster.linear_impulse_scale,
            })
            .collect::<Vec<_>>();
        let vertex_count = vertices.len();
        let target_vertices = configuration.vertices.clone();
        let initial_medium_velocity = configuration.initial_medium_velocity;
        let mut state = Self {
            configuration,
            query_collider,
            previous_vertices: vertices.clone(),
            velocities,
            normals: vec![Vec3::ZERO; vertex_count],
            vertex_areas: vec![0.0; vertex_count],
            target_host: pose,
            target_vertices,
            target_normals: vec![Vec3::ZERO; vertex_count],
            target_active: false,
            pose_frame: pose,
            medium_velocity: initial_medium_velocity,
            previous_medium_velocity: initial_medium_velocity,
            target_medium_velocity: initial_medium_velocity,
            medium_velocity_elapsed: 0.0,
            medium_velocity_duration: 0.0,
            medium_velocity_random_state: 0,
            vertices,
            awake: true,
            entity_pose: pose,
            inverse_masses: vec![0.0; vertex_count],
            links,
            secondary_links,
            clusters,
            pending_force: Vec3::ZERO,
            pending_torque: Vec3::ZERO,
            reactions: SmallVec::new(),
        };
        state.recalculate_inverse_masses();
        state.recalculate_normals();
        state.recalculate_target_normals();
        state
    }

    pub(crate) fn set_pose(&mut self, pose: PhysicsPose) {
        let delta = pose * self.entity_pose.inverse();
        for vertex in &mut self.vertices {
            *vertex = delta.transform_point(*vertex);
        }
        for vertex in &mut self.previous_vertices {
            *vertex = delta.transform_point(*vertex);
        }
        for velocity in &mut self.velocities {
            *velocity = delta.rotation * *velocity;
        }
        self.target_host = delta * self.target_host;
        self.pose_frame = delta * self.pose_frame;
        self.entity_pose = pose;
        self.recalculate_normals();
        self.awake = true;
    }

    pub(crate) fn set_target(
        &mut self,
        target: DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        target.validate()?;
        if target
            .points
            .as_ref()
            .is_some_and(|points| points.len() != self.vertices.len())
        {
            return Err(PhysicsError::InvalidLinkedSoftBodyConfiguration {
                field: "target vertex count",
            });
        }

        let host = target.host.unwrap_or(PhysicsPose::IDENTITY);
        let inverse_host = host.inverse();
        if let Some(points) = target.points {
            for (destination, point) in self.target_vertices.iter_mut().zip(points) {
                *destination = inverse_host.transform_point(point);
            }
        } else {
            for (destination, point) in self.target_vertices.iter_mut().zip(&self.vertices) {
                *destination = inverse_host.transform_point(*point);
            }
        }
        self.target_host = host;
        self.target_active = true;
        self.recalculate_target_normals();
        self.awake = true;
        Ok(())
    }

    pub(crate) fn reset(&mut self) {
        for (vertex, source) in self.vertices.iter_mut().zip(&self.configuration.vertices) {
            *vertex = self.entity_pose.transform_point(*source);
        }
        self.previous_vertices.clone_from(&self.vertices);
        if self.configuration.velocities.is_empty() {
            self.velocities.fill(Vec3::ZERO);
        } else {
            for (velocity, source) in self
                .velocities
                .iter_mut()
                .zip(&self.configuration.velocities)
            {
                *velocity = self.entity_pose.rotation * *source;
            }
        }
        self.pending_force = Vec3::ZERO;
        self.pending_torque = Vec3::ZERO;
        self.pose_frame = self.entity_pose;
        self.medium_velocity = self.configuration.initial_medium_velocity;
        self.previous_medium_velocity = self.medium_velocity;
        self.target_medium_velocity = self.medium_velocity;
        self.medium_velocity_elapsed = 0.0;
        self.medium_velocity_duration = 0.0;
        self.medium_velocity_random_state = 0;
        self.reactions.clear();
        self.recalculate_normals();
        self.awake = true;
    }

    pub(crate) fn set_velocity(&mut self, velocity: Vec3) {
        self.velocities.fill(velocity);
        self.awake = true;
    }

    pub(crate) fn set_angular_velocity(&mut self, angular_velocity: Vec3) {
        let (center, _) = deformable_bounds(&self.vertices);
        for (velocity, vertex) in self.velocities.iter_mut().zip(&self.vertices) {
            *velocity = angular_velocity.cross(*vertex - center);
        }
        self.awake = true;
    }

    pub(crate) fn apply_angular_impulse(&mut self, impulse: Vec3) {
        let (_, radius) = deformable_bounds(&self.vertices);
        let inertia = (self.configuration.mass * radius * radius).max(1.0e-6);
        let angular_velocity = impulse / inertia;
        let (center, _) = deformable_bounds(&self.vertices);
        for (velocity, vertex) in self.velocities.iter_mut().zip(&self.vertices) {
            *velocity += angular_velocity.cross(*vertex - center);
        }
        self.awake = true;
    }

    pub(crate) fn apply_impulse(&mut self, action: ImpulseAction) {
        let point = action
            .point
            .unwrap_or_else(|| deformable_bounds(&self.vertices).0);
        let nearest = self
            .vertices
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| {
                left.distance_squared(point)
                    .total_cmp(&right.distance_squared(point))
            })
            .map_or(0, |(index, _)| index);
        self.velocities[nearest] += action.impulse * self.inverse_masses[nearest];
        self.awake = true;
    }

    pub(crate) fn add_force(&mut self, force: Vec3) {
        self.pending_force += force;
        self.awake = true;
    }

    pub(crate) fn add_torque(&mut self, torque: Vec3) {
        self.pending_torque += torque;
        self.awake = true;
    }

    pub(crate) fn set_mass(&mut self, mass: f32) {
        self.configuration.mass = mass;
        self.recalculate_inverse_masses();
        self.awake = true;
    }

    pub(crate) fn status(&self) -> LinkedSoftBodyStatusRef<'_> {
        LinkedSoftBodyStatusRef {
            vertices: &self.vertices,
            velocities: &self.velocities,
            normals: &self.normals,
            faces: &self.configuration.faces,
            awake: self.awake,
        }
    }

    pub(crate) fn take_reactions(&mut self) -> SmallVec<[DeformableReaction; 16]> {
        core::mem::take(&mut self.reactions)
    }

    pub(crate) fn step<C, M>(
        &mut self,
        time_step: f32,
        world_gravity: Vec3,
        mut collide: C,
        mut medium_at: M,
    ) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
        M: FnMut(Vec3) -> MediumSample,
    {
        self.reactions.clear();
        self.previous_vertices.clone_from(&self.vertices);
        self.recalculate_normals();

        let vertex_mass = self.configuration.mass / f32_from_usize(self.vertices.len());
        let shared_acceleration = self.pending_force / self.configuration.mass;
        self.pending_force = Vec3::ZERO;
        let torque = core::mem::take(&mut self.pending_torque);
        if torque != Vec3::ZERO {
            self.apply_angular_impulse(torque * time_step);
        }
        for velocity in &mut self.velocities {
            *velocity += (world_gravity * self.configuration.gravity_factor + shared_acceleration)
                * time_step;
        }

        self.apply_pressure_and_volume_forces(time_step, vertex_mass);
        self.apply_aerodynamic_forces(time_step, vertex_mass, &mut medium_at);
        self.apply_target_position_forces(time_step);
        self.apply_pose_frame_delta();

        for (vertex, velocity) in self.vertices.iter_mut().zip(&self.velocities) {
            *vertex += *velocity * time_step;
        }
        self.apply_rest_pose_projection(time_step);
        self.solve_links();
        self.solve_target_envelopes();

        if self.configuration.rigid_collision_feature == LinkedSoftBodyCollisionFeature::Vertex {
            for index in 0..self.vertices.len() {
                let Some(contact) = collide(
                    self.previous_vertices[index],
                    self.vertices[index],
                    self.configuration.collision_radius,
                )?
                else {
                    self.velocities[index] =
                        (self.vertices[index] - self.previous_vertices[index]) / time_step;
                    continue;
                };
                let free_velocity =
                    (self.vertices[index] - self.previous_vertices[index]) / time_step;
                self.vertices[index] =
                    contact.position + contact.normal * self.configuration.collision_radius;
                let mut velocity =
                    (self.vertices[index] - self.previous_vertices[index]) / time_step;
                let relative = velocity - contact.velocity;
                let normal_speed = relative.dot(contact.normal);
                if normal_speed < 0.0 {
                    let normal_change =
                        contact.normal * (-(1.0 + self.configuration.restitution) * normal_speed);
                    velocity += normal_change;
                    let tangent = relative - contact.normal * normal_speed;
                    let maximum_friction = normal_change.length() * self.configuration.friction;
                    velocity -=
                        tangent.normalize_or_zero() * tangent.length().min(maximum_friction);
                }
                if contact.dynamic {
                    let reaction = -(velocity - free_velocity) * vertex_mass;
                    if reaction.is_finite() && reaction != Vec3::ZERO {
                        self.reactions.push(DeformableReaction {
                            body: contact.body,
                            point: contact.position,
                            impulse: reaction,
                        });
                    }
                }
                self.velocities[index] = velocity;
            }
        }
        match self.configuration.rigid_collision_feature {
            LinkedSoftBodyCollisionFeature::Cluster => {
                self.resolve_cluster_contacts(time_step, &mut collide)?;
            }
            LinkedSoftBodyCollisionFeature::Face => {
                self.resolve_face_contacts(time_step, &mut collide)?;
            }
            LinkedSoftBodyCollisionFeature::Vertex => {}
        }

        self.recalculate_normals();
        self.advance_medium_velocity(time_step);
        let kinetic_energy = self
            .velocities
            .iter()
            .map(|velocity| velocity.length_squared())
            .sum::<f32>()
            * (0.5 * vertex_mass);
        self.awake = kinetic_energy > self.configuration.minimum_energy;
        Ok(())
    }

    /// Maps `RockNRoll`'s aggregate deformable feature, `kind == 2`, onto
    /// Rapier shape casts.
    ///
    /// The native solver derives a center, linear velocity, angular velocity,
    /// and inverse inertia for every configured member set, projects a contact
    /// impulse, then applies `linear + angular × offset` to every member.
    /// Rapier supplies the contact; this method preserves that aggregate
    /// response instead of treating the covered vertices as unrelated
    /// particles.
    fn resolve_face_contacts<C>(
        &mut self,
        time_step: f32,
        collide: &mut C,
    ) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        if self.configuration.faces.is_empty()
            || self.configuration.maximum_rigid_contact_solve_iterations == 0
        {
            return Ok(());
        }
        for _ in 0..self.configuration.maximum_rigid_contact_solve_iterations {
            for &face in &self.configuration.faces {
                let indices = face.map(|index| index as usize);
                let previous = indices.map(|index| self.previous_vertices[index]);
                let current = indices.map(|index| self.vertices[index]);
                let previous_center = previous.into_iter().sum::<Vec3>() / 3.0;
                let center = current.into_iter().sum::<Vec3>() / 3.0;
                let radius = current
                    .into_iter()
                    .map(|vertex| vertex.distance(center))
                    .fold(self.configuration.collision_radius, f32::max);
                let Some(contact) = collide(previous_center, center, radius)? else {
                    continue;
                };
                let normal = contact.normal.normalize_or_zero();
                if normal == Vec3::ZERO {
                    continue;
                }
                let free_velocity = (center - previous_center) / time_step;
                let relative_velocity = free_velocity - contact.velocity;
                let face_inverse_mass = indices
                    .iter()
                    .map(|&index| self.inverse_masses[index])
                    .sum::<f32>()
                    / 9.0;
                let impulse = linked_soft_body_project_contact_impulse(
                    relative_velocity,
                    normal,
                    self.configuration.restitution,
                    self.configuration.friction,
                    face_inverse_mass,
                    face_inverse_mass,
                );
                for index in indices {
                    let delta = impulse * (self.inverse_masses[index] / 3.0);
                    self.velocities[index] += delta;
                    self.vertices[index] += delta * time_step;
                }
                let target_center = contact.position + normal * radius;
                let penetration = (target_center - center).dot(normal).max(0.0);
                if penetration > 0.0 && face_inverse_mass > f32::EPSILON {
                    let correction = normal * (penetration / face_inverse_mass);
                    for index in indices {
                        self.vertices[index] += correction * (self.inverse_masses[index] / 3.0);
                    }
                }
                if contact.dynamic && impulse != Vec3::ZERO {
                    self.reactions.push(DeformableReaction {
                        body: contact.body,
                        point: contact.position,
                        impulse: -impulse,
                    });
                }
            }
        }
        Ok(())
    }

    fn resolve_cluster_contacts<C>(
        &mut self,
        time_step: f32,
        collide: &mut C,
    ) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        if self.clusters.is_empty()
            || self.configuration.maximum_rigid_contact_solve_iterations == 0
        {
            return Ok(());
        }

        let vertex_mass = self.configuration.mass / f32_from_usize(self.vertices.len());
        for _ in 0..self.configuration.maximum_rigid_contact_solve_iterations {
            let mut accumulated_positions = self
                .configuration
                .accumulate_cluster_corrections
                .then(|| vec![Vec3::ZERO; self.vertices.len()]);
            for cluster_index in 0..self.clusters.len() {
                let frame = linked_soft_body_cluster_frame(
                    &self.vertices,
                    &self.previous_vertices,
                    &self.clusters[cluster_index],
                    vertex_mass,
                    time_step,
                );
                let query_radius = frame.radius + self.configuration.collision_radius;
                let Some(contact) = collide(frame.previous_center, frame.center, query_radius)?
                else {
                    continue;
                };

                let cluster = &self.clusters[cluster_index];
                let original_positions = accumulated_positions.as_ref().map(|_| {
                    cluster
                        .vertices
                        .iter()
                        .map(|&vertex| self.vertices[vertex])
                        .collect::<SmallVec<[Vec3; 16]>>()
                });
                let normal = contact.normal.normalize_or_zero();
                if normal == Vec3::ZERO {
                    continue;
                }
                let (impulse, normal_inverse_mass) = linked_soft_body_cluster_rigid_impulse(
                    &self.configuration,
                    cluster,
                    frame,
                    contact,
                    normal,
                );

                linked_soft_body_apply_cluster_velocity_impulse(
                    &mut self.vertices,
                    &mut self.velocities,
                    cluster,
                    frame,
                    contact.position,
                    impulse,
                    time_step,
                );

                let target_center = contact.position + normal * query_radius;
                let position_error = (target_center - frame.center).dot(normal).max(0.0);
                if position_error > 0.0 && normal_inverse_mass > f32::EPSILON {
                    linked_soft_body_apply_cluster_position_impulse(
                        &mut self.vertices,
                        cluster,
                        frame,
                        contact.position,
                        normal * (position_error / normal_inverse_mass),
                    );
                }

                if contact.dynamic && impulse != Vec3::ZERO {
                    self.reactions.push(DeformableReaction {
                        body: contact.body,
                        point: contact.position,
                        impulse: -impulse,
                    });
                }
                if let (Some(accumulated), Some(original)) =
                    (accumulated_positions.as_mut(), original_positions)
                {
                    for (&vertex, original) in cluster.vertices.iter().zip(original) {
                        accumulated[vertex] += self.vertices[vertex] - original;
                        self.vertices[vertex] = original;
                    }
                }
            }
            if let Some(accumulated) = accumulated_positions {
                for (vertex, correction) in self.vertices.iter_mut().zip(accumulated) {
                    *vertex += correction;
                }
            }
        }
        Ok(())
    }

    fn recalculate_inverse_masses(&mut self) {
        let inverse_mass = f32_from_usize(self.vertices.len()) / self.configuration.mass;
        self.inverse_masses.fill(inverse_mass);
    }

    fn solve_links(&mut self) {
        let minimum_squared_ratio = self.configuration.minimum_link_length_squared_ratio;
        for _ in 0..self.configuration.maximum_link_solve_iterations {
            solve_link_family(
                &mut self.vertices,
                &self.inverse_masses,
                &self.links,
                self.configuration.link_stiffness_coefficient,
                minimum_squared_ratio,
            );
            solve_link_family(
                &mut self.vertices,
                &self.inverse_masses,
                &self.secondary_links,
                self.configuration.secondary_link_stiffness_coefficient,
                minimum_squared_ratio,
            );
        }
    }

    /// Applies the two rational squared-distance projections in solver order.
    fn solve_target_envelopes(&mut self) {
        if !self.target_active || self.configuration.vertex_envelopes.is_empty() {
            return;
        }

        for _ in 0..self.configuration.maximum_target_solve_iterations {
            for index in 0..self.vertices.len() {
                let envelope = self.configuration.vertex_envelopes[index];
                let target = self
                    .target_host
                    .transform_point(self.target_vertices[index]);
                let normal = self.target_host.rotation * self.target_normals[index];

                let maximum_center = target - envelope.normal_offset.min(0.0) * normal;
                let maximum_delta = maximum_center - self.vertices[index];
                let maximum_distance_squared = maximum_delta.length_squared();
                let maximum_radius_squared = envelope.maximum_distance * envelope.maximum_distance;
                let maximum_denominator = maximum_distance_squared + maximum_radius_squared;
                if maximum_denominator > f32::EPSILON {
                    let correction = (maximum_distance_squared - maximum_radius_squared).max(0.0)
                        / maximum_denominator;
                    self.vertices[index] += correction * maximum_delta;
                }

                let minimum_center =
                    target - (envelope.normal_offset + envelope.minimum_distance) * normal;
                let minimum_delta = self.vertices[index] - minimum_center;
                let minimum_distance_squared = minimum_delta.length_squared();
                let minimum_radius_squared = envelope.minimum_distance * envelope.minimum_distance;
                let minimum_denominator = minimum_distance_squared + minimum_radius_squared;
                if minimum_denominator > f32::EPSILON {
                    let correction = (minimum_radius_squared - minimum_distance_squared).max(0.0)
                        / minimum_denominator;
                    self.vertices[index] += correction * minimum_delta;
                }
            }
        }
    }

    /// Applies the target-position branch. The solver predicts a free
    /// position, applies a spring toward the active target, and subtracts the
    /// preceding displacement scaled by the target-only damping coefficient.
    /// Expressing that position update as a velocity delta preserves the same
    /// `dt²` integration without a second predicted-position buffer.
    fn apply_target_position_forces(&mut self, time_step: f32) {
        let stiffness = self.configuration.target_position_coefficient;
        if !self.target_active || stiffness <= 0.0 {
            return;
        }

        let damping = self.configuration.target_position_damping_coefficient;
        for index in 0..self.vertices.len() {
            let predicted = self.vertices[index] + self.velocities[index] * time_step;
            let target = self
                .target_host
                .transform_point(self.target_vertices[index]);
            let acceleration = (target - predicted) * stiffness - self.velocities[index] * damping;
            self.velocities[index] += acceleration * time_step;
        }
    }

    /// Advances vertices through the computed pose-frame delta.
    ///
    /// The solver converts the frame delta to a quaternion, interpolates from
    /// identity, scales its translation by the same coefficient, and applies
    /// that transform before ordinary displacement.
    fn apply_pose_frame_delta(&mut self) {
        let coefficient = self.configuration.pose_matching_coefficient;
        if coefficient <= 0.0 {
            return;
        }
        let Some(current_frame) = fit_linked_soft_body_pose(
            &self.configuration.vertices,
            &self.vertices,
            self.pose_frame.rotation,
        ) else {
            return;
        };
        let frame_delta = current_frame * self.pose_frame.inverse();
        let interpolated_delta = PhysicsPose {
            translation: frame_delta.translation * coefficient,
            rotation: Quat::IDENTITY
                .slerp(frame_delta.rotation, coefficient)
                .normalize(),
        };
        for vertex in &mut self.vertices {
            *vertex = interpolated_delta.transform_point(*vertex);
        }
        self.pose_frame = current_frame;
    }

    /// Projects predicted vertices toward the velocity-advanced rest pose.
    ///
    /// Its blend is `min(rate * dt, 1)`. The target combines a pose-transformed
    /// authored vertex with the vertex's preceding displacement.
    fn apply_rest_pose_projection(&mut self, time_step: f32) {
        let weight = (self.configuration.rest_pose_projection_rate * time_step).min(1.0);
        if weight <= 0.0 {
            return;
        }
        let Some(frame) = fit_linked_soft_body_pose(
            &self.configuration.vertices,
            &self.previous_vertices,
            self.pose_frame.rotation,
        ) else {
            return;
        };
        for index in 0..self.vertices.len() {
            let displacement = self.velocities[index] * time_step;
            let target = frame.transform_point(self.configuration.vertices[index]) + displacement;
            self.vertices[index] = self.vertices[index].lerp(target, weight);
        }
        self.pose_frame = frame;
    }

    fn apply_pressure_and_volume_forces(&mut self, time_step: f32, vertex_mass: f32) {
        if self.configuration.faces.is_empty()
            || (self.configuration.pressure_coefficient == 0.0
                && self.configuration.volume_maintenance_factor == 0.0)
        {
            return;
        }
        let volume = linked_soft_body_volume(&self.vertices, &self.configuration.faces);
        if volume.abs() <= f32::EPSILON {
            return;
        }
        let pressure = (self.configuration.desired_volume - volume).mul_add(
            self.configuration.volume_maintenance_factor,
            self.configuration.pressure_coefficient / volume,
        );
        for &face in &self.configuration.faces {
            let [a, b, c] = face.map(|index| index as usize);
            let area_normal = (self.vertices[b] - self.vertices[a])
                .cross(self.vertices[c] - self.vertices[a])
                * 0.5;
            let acceleration = area_normal * (pressure / (3.0 * vertex_mass));
            for index in [a, b, c] {
                self.velocities[index] += acceleration * time_step;
            }
        }
    }

    fn apply_aerodynamic_forces<M>(&mut self, time_step: f32, vertex_mass: f32, medium_at: &mut M)
    where
        M: FnMut(Vec3) -> MediumSample,
    {
        let model = self.configuration.aerodynamics;
        if model == LinkedSoftBodyAerodynamics::None {
            return;
        }
        if matches!(
            model,
            LinkedSoftBodyAerodynamics::FaceTwoSidedLiftDrag
                | LinkedSoftBodyAerodynamics::FaceOneSidedLiftDrag
        ) {
            let one_sided = model == LinkedSoftBodyAerodynamics::FaceOneSidedLiftDrag;
            for &face in &self.configuration.faces {
                let indices = face.map(|index| index as usize);
                let [a, b, c] = indices.map(|index| self.vertices[index]);
                let area_normal = (b - a).cross(c - a) * 0.5;
                let area = area_normal.length();
                if area <= f32::EPSILON {
                    continue;
                }
                let center = (a + b + c) / 3.0;
                let medium = medium_at(center);
                let reference_velocity = if self.configuration.aerodynamics_in_pose_space {
                    self.pose_frame.rotation * self.medium_velocity
                } else {
                    self.medium_velocity
                };
                let flow = reference_velocity + medium.velocity;
                let velocity = indices
                    .iter()
                    .map(|&index| self.velocities[index])
                    .sum::<Vec3>()
                    / 3.0;
                let density = if medium.submerged_fraction > 0.0 {
                    medium.water_density
                } else {
                    self.configuration.medium_density
                };
                let force = lift_drag_force(
                    area_normal / area,
                    velocity - flow,
                    area,
                    density,
                    self.configuration.air_friction_lift,
                    self.configuration.air_friction_drag,
                    one_sided,
                );
                let velocity_delta = force * (time_step / (3.0 * vertex_mass));
                for index in indices {
                    self.velocities[index] += velocity_delta;
                }
            }
            return;
        }

        let one_sided = model == LinkedSoftBodyAerodynamics::VertexOneSidedLiftDrag;
        for index in 0..self.vertices.len() {
            let medium = medium_at(self.vertices[index]);
            let density = if medium.submerged_fraction > 0.0 {
                medium.water_density
            } else {
                self.configuration.medium_density
            };
            let reference_velocity = if self.configuration.aerodynamics_in_pose_space {
                self.pose_frame.rotation * self.medium_velocity
            } else {
                self.medium_velocity
            };
            let relative = self.velocities[index] - (reference_velocity + medium.velocity);
            let force = if model == LinkedSoftBodyAerodynamics::VertexPoint {
                -relative
                    * (relative.length()
                        * density
                        * self.configuration.air_friction_drag
                        * self.vertex_areas[index])
            } else {
                lift_drag_force(
                    self.normals[index],
                    relative,
                    self.vertex_areas[index],
                    density,
                    self.configuration.air_friction_lift,
                    self.configuration.air_friction_drag,
                    one_sided,
                )
            };
            self.velocities[index] += force * (time_step / vertex_mass);
        }
    }

    /// Advances the medium-velocity state using the 48-bit LCG after the
    /// force and constraint pass. The new value becomes the aerodynamic
    /// reference velocity for the next substep.
    fn advance_medium_velocity(&mut self, time_step: f32) {
        let Some(animation) = self.configuration.medium_velocity_animation else {
            self.medium_velocity = self.configuration.initial_medium_velocity;
            return;
        };
        self.medium_velocity_elapsed += time_step;
        match animation.mode {
            LinkedSoftBodyMediumVelocityMode::InterpolatedRandom => {
                if self.medium_velocity_elapsed >= self.medium_velocity_duration {
                    self.previous_medium_velocity = self.medium_velocity;
                    self.target_medium_velocity = Vec3::new(
                        linked_soft_body_random_range(
                            &mut self.medium_velocity_random_state,
                            animation.minimum_velocity.x,
                            animation.maximum_velocity.x,
                        ),
                        linked_soft_body_random_range(
                            &mut self.medium_velocity_random_state,
                            animation.minimum_velocity.y,
                            animation.maximum_velocity.y,
                        ),
                        linked_soft_body_random_range(
                            &mut self.medium_velocity_random_state,
                            animation.minimum_velocity.z,
                            animation.maximum_velocity.z,
                        ),
                    );
                    self.medium_velocity_elapsed = time_step;
                    self.medium_velocity_duration = linked_soft_body_random_range(
                        &mut self.medium_velocity_random_state,
                        animation.minimum_duration,
                        animation.maximum_duration,
                    );
                }
                let fraction =
                    (self.medium_velocity_elapsed / self.medium_velocity_duration).clamp(0.0, 1.0);
                self.medium_velocity = self
                    .previous_medium_velocity
                    .lerp(self.target_medium_velocity, fraction);
            }
            LinkedSoftBodyMediumVelocityMode::AlternatingStep => {
                if self.medium_velocity_elapsed >= self.medium_velocity_duration {
                    self.medium_velocity = if self.medium_velocity == animation.minimum_velocity {
                        animation.maximum_velocity
                    } else {
                        animation.minimum_velocity
                    };
                    self.medium_velocity_elapsed = 0.0;
                    self.medium_velocity_duration = linked_soft_body_random_range(
                        &mut self.medium_velocity_random_state,
                        animation.minimum_duration,
                        animation.maximum_duration,
                    );
                }
            }
        }
    }

    fn recalculate_normals(&mut self) {
        self.normals.fill(Vec3::ZERO);
        self.vertex_areas.fill(0.0);
        for &face in &self.configuration.faces {
            let [a, b, c] = face.map(|index| index as usize);
            let normal =
                (self.vertices[b] - self.vertices[a]).cross(self.vertices[c] - self.vertices[a]);
            self.normals[a] += normal;
            self.normals[b] += normal;
            self.normals[c] += normal;
            let area = normal.length() * 0.5 / 3.0;
            self.vertex_areas[a] += area;
            self.vertex_areas[b] += area;
            self.vertex_areas[c] += area;
        }
        for normal in &mut self.normals {
            *normal = normal.normalize_or_zero();
        }
    }

    fn recalculate_target_normals(&mut self) {
        calculate_vertex_normals(
            &self.target_vertices,
            &self.configuration.faces,
            &mut self.target_normals,
        );
    }
}

/// Fits the authored linked-soft rest vertices to a world-space vertex set.
///
/// The solver keeps a rigid pose frame for pose matching and rest-pose
/// projection. Horn's quaternion form of the orthogonal Procrustes fit gives
/// the same rigid-frame product without allocating a matrix decomposition and
/// remains well-defined for planar triangle meshes.
fn fit_linked_soft_body_pose(
    rest_vertices: &[Vec3],
    world_vertices: &[Vec3],
    rotation_hint: Quat,
) -> Option<PhysicsPose> {
    if rest_vertices.len() != world_vertices.len() || rest_vertices.len() < 2 {
        return None;
    }

    let inverse_count = 1.0 / f32_from_usize(rest_vertices.len());
    let rest_center = rest_vertices.iter().copied().sum::<Vec3>() * inverse_count;
    let world_center = world_vertices.iter().copied().sum::<Vec3>() * inverse_count;

    let mut covariance = [[0.0_f32; 3]; 3];
    for (&rest, &world) in rest_vertices.iter().zip(world_vertices) {
        let rest = rest - rest_center;
        let world = world - world_center;
        covariance[0][0] = rest.x.mul_add(world.x, covariance[0][0]);
        covariance[0][1] = rest.x.mul_add(world.y, covariance[0][1]);
        covariance[0][2] = rest.x.mul_add(world.z, covariance[0][2]);
        covariance[1][0] = rest.y.mul_add(world.x, covariance[1][0]);
        covariance[1][1] = rest.y.mul_add(world.y, covariance[1][1]);
        covariance[1][2] = rest.y.mul_add(world.z, covariance[1][2]);
        covariance[2][0] = rest.z.mul_add(world.x, covariance[2][0]);
        covariance[2][1] = rest.z.mul_add(world.y, covariance[2][1]);
        covariance[2][2] = rest.z.mul_add(world.z, covariance[2][2]);
    }
    let [[sxx, sxy, sxz], [syx, syy, syz], [szx, szy, szz]] = covariance;
    let trace = sxx + syy + szz;
    let matrix = [
        [trace, syz - szy, szx - sxz, sxy - syx],
        [syz - szy, sxx - syy - szz, sxy + syx, szx + sxz],
        [szx - sxz, sxy + syx, -sxx + syy - szz, syz + szy],
        [sxy - syx, szx + sxz, syz + szy, -sxx - syy + szz],
    ];
    let shift = matrix
        .iter()
        .flatten()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt()
        + f32::EPSILON;
    let hint = rotation_hint.normalize();
    let mut quaternion = Vec4::new(hint.w, hint.x, hint.y, hint.z);
    for _ in 0..16 {
        let next = Vec4::new(
            shift.mul_add(
                quaternion.x,
                matrix[0][3].mul_add(
                    quaternion.w,
                    matrix[0][2].mul_add(
                        quaternion.z,
                        matrix[0][1].mul_add(quaternion.y, matrix[0][0] * quaternion.x),
                    ),
                ),
            ),
            shift.mul_add(
                quaternion.y,
                matrix[1][3].mul_add(
                    quaternion.w,
                    matrix[1][2].mul_add(
                        quaternion.z,
                        matrix[1][1].mul_add(quaternion.y, matrix[1][0] * quaternion.x),
                    ),
                ),
            ),
            shift.mul_add(
                quaternion.z,
                matrix[2][3].mul_add(
                    quaternion.w,
                    matrix[2][2].mul_add(
                        quaternion.z,
                        matrix[2][1].mul_add(quaternion.y, matrix[2][0] * quaternion.x),
                    ),
                ),
            ),
            shift.mul_add(
                quaternion.w,
                matrix[3][3].mul_add(
                    quaternion.w,
                    matrix[3][2].mul_add(
                        quaternion.z,
                        matrix[3][1].mul_add(quaternion.y, matrix[3][0] * quaternion.x),
                    ),
                ),
            ),
        );
        if next.length_squared() <= f32::EPSILON || !next.is_finite() {
            return None;
        }
        quaternion = next.normalize();
    }
    let rotation =
        Quat::from_xyzw(quaternion.y, quaternion.z, quaternion.w, quaternion.x).normalize();
    let translation = world_center - rotation * rest_center;
    (rotation.is_finite() && translation.is_finite()).then_some(PhysicsPose {
        translation,
        rotation,
    })
}

/// Solves `RockNRoll`'s deformable-to-deformable feature records for a pair of
/// linked soft bodies.
///
/// The `RockNRoll` solver accepts vertex (`0`), face-average (`1`), and aggregate
/// cluster (`2`) features and applies equal-and-opposite projected impulses.
/// This Rapier adapter performs the narrow phase over the authored topology,
/// while retaining those three response representations and the native
/// per-pair maximum iteration rule.
pub fn solve_linked_soft_body_pair(
    left: &mut LinkedSoftBodyState,
    right: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let iterations = left
        .configuration
        .maximum_deformable_contact_solve_iterations
        .max(
            right
                .configuration
                .maximum_deformable_contact_solve_iterations,
        );
    if iterations == 0 {
        return;
    }

    for _ in 0..iterations {
        match (
            left.configuration.deformable_collision_feature,
            right.configuration.deformable_collision_feature,
        ) {
            (LinkedSoftBodyCollisionFeature::Vertex, LinkedSoftBodyCollisionFeature::Vertex) => {
                solve_linked_soft_body_vertex_contacts(left, right, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Vertex, LinkedSoftBodyCollisionFeature::Face) => {
                solve_linked_soft_body_vertex_face_contacts(left, right, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Face, LinkedSoftBodyCollisionFeature::Vertex) => {
                solve_linked_soft_body_vertex_face_contacts(right, left, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Cluster, LinkedSoftBodyCollisionFeature::Face) => {
                solve_linked_soft_body_cluster_face_contacts(left, right, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Face, LinkedSoftBodyCollisionFeature::Cluster) => {
                solve_linked_soft_body_cluster_face_contacts(right, left, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Cluster, LinkedSoftBodyCollisionFeature::Cluster) => {
                solve_linked_soft_body_cluster_contacts(left, right, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Cluster, LinkedSoftBodyCollisionFeature::Vertex) => {
                solve_linked_soft_body_cluster_vertex_contacts(left, right, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Vertex, LinkedSoftBodyCollisionFeature::Cluster) => {
                solve_linked_soft_body_cluster_vertex_contacts(right, left, time_step);
            }
            (LinkedSoftBodyCollisionFeature::Face, LinkedSoftBodyCollisionFeature::Face) => {
                solve_linked_soft_body_face_contacts(left, right, time_step);
            }
        }
    }
    left.recalculate_normals();
    right.recalculate_normals();
}

fn solve_linked_soft_body_vertex_face_contacts(
    vertex_body: &mut LinkedSoftBodyState,
    face_body: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let radius =
        vertex_body.configuration.collision_radius + face_body.configuration.collision_radius;
    let radius_squared = radius * radius;
    for vertex in 0..vertex_body.vertices.len() {
        let point = vertex_body.vertices[vertex];
        let nearest = face_body
            .configuration
            .faces
            .iter()
            .enumerate()
            .map(|(face_index, &face)| {
                let triangle = face.map(|index| face_body.vertices[index as usize]);
                let (weights, distance_squared) = closest_triangle_weights(point, triangle);
                (face_index, weights, distance_squared)
            })
            .filter(|(_, _, distance_squared)| *distance_squared < radius_squared)
            .min_by(|left, right| left.2.total_cmp(&right.2));
        let Some((face_index, weights, distance_squared)) = nearest else {
            continue;
        };
        let face = face_body.configuration.faces[face_index].map(|index| index as usize);
        let triangle = face.map(|index| face_body.vertices[index]);
        let closest =
            triangle[0] * weights[0] + triangle[1] * weights[1] + triangle[2] * weights[2];
        let face_normal = (triangle[1] - triangle[0])
            .cross(triangle[2] - triangle[0])
            .normalize_or_zero();
        let separation = point - closest;
        let normal = if separation.length_squared() > f32::EPSILON {
            separation.normalize()
        } else {
            let relative = vertex_body.velocities[vertex]
                - face
                    .iter()
                    .map(|&index| face_body.velocities[index])
                    .sum::<Vec3>()
                    / 3.0;
            if relative.dot(face_normal) <= 0.0 {
                face_normal
            } else {
                -face_normal
            }
        };
        if normal == Vec3::ZERO {
            continue;
        }
        let face_velocity = face
            .iter()
            .map(|&index| face_body.velocities[index])
            .sum::<Vec3>()
            / 3.0;
        let relative_velocity = vertex_body.velocities[vertex] - face_velocity;
        let vertex_inverse_mass = vertex_body.inverse_masses[vertex];
        let face_inverse_mass = face
            .iter()
            .map(|&index| face_body.inverse_masses[index])
            .sum::<f32>()
            / 9.0;
        let inverse_mass = vertex_inverse_mass + face_inverse_mass;
        let impulse = linked_soft_body_project_contact_impulse(
            relative_velocity,
            normal,
            0.5 * (vertex_body.configuration.restitution + face_body.configuration.restitution),
            vertex_body.configuration.friction * face_body.configuration.friction,
            inverse_mass,
            inverse_mass,
        );
        vertex_body.velocities[vertex] += impulse * vertex_inverse_mass;
        vertex_body.vertices[vertex] += impulse * vertex_inverse_mass * time_step;
        for index in face {
            let delta = impulse * (face_body.inverse_masses[index] / 3.0);
            face_body.velocities[index] -= delta;
            face_body.vertices[index] -= delta * time_step;
        }

        let penetration = radius - distance_squared.sqrt();
        if penetration > 0.0 && inverse_mass > f32::EPSILON {
            let correction_impulse = normal * (penetration / inverse_mass);
            vertex_body.vertices[vertex] += correction_impulse * vertex_inverse_mass;
            for index in face {
                face_body.vertices[index] -=
                    correction_impulse * (face_body.inverse_masses[index] / 3.0);
            }
        }
    }
}

/// Nearest face of `body` to `point` inside `radius_squared`, as the face's
/// vertex indices, the barycentric weights of the closest point on it, and that
/// squared distance.
fn nearest_linked_soft_body_face(
    body: &LinkedSoftBodyState,
    point: Vec3,
    radius_squared: f32,
) -> Option<([usize; 3], [f32; 3], f32)> {
    body.configuration
        .faces
        .iter()
        .enumerate()
        .map(|(face_index, &face)| {
            let triangle = face.map(|index| body.vertices[index as usize]);
            let (weights, distance_squared) = closest_triangle_weights(point, triangle);
            (face_index, weights, distance_squared)
        })
        .filter(|(_, _, distance_squared)| *distance_squared < radius_squared)
        .min_by(|left, right| left.2.total_cmp(&right.2))
        .map(|(face_index, weights, distance_squared)| {
            (
                body.configuration.faces[face_index].map(|index| index as usize),
                weights,
                distance_squared,
            )
        })
}

/// Mean velocity of a face's three vertices.
fn linked_soft_body_face_velocity(body: &LinkedSoftBodyState, face: [usize; 3]) -> Vec3 {
    face.iter()
        .map(|&index| body.velocities[index])
        .sum::<Vec3>()
        / 3.0
}

/// Inverse mass of a face treated as one contact feature.
fn linked_soft_body_face_inverse_mass(body: &LinkedSoftBodyState, face: [usize; 3]) -> f32 {
    face.iter()
        .map(|&index| body.inverse_masses[index])
        .sum::<f32>()
        / 9.0
}

/// Impulse between an aggregate cluster and a face of another deformable,
/// together with the normal-direction inverse mass the position pass reuses.
fn linked_soft_body_cluster_face_impulse(
    cluster_body: &LinkedSoftBodyState,
    face_body: &LinkedSoftBodyState,
    cluster: &LinkedSoftBodyCluster,
    frame: LinkedSoftBodyClusterFrame,
    face: [usize; 3],
    point: Vec3,
    normal: Vec3,
) -> (Vec3, f32) {
    let contact_offset = point - frame.center;
    let cluster_inverse_mass = linked_soft_body_cluster_inverse_mass(
        cluster.linear_impulse_scale,
        frame.inverse_inertia,
        contact_offset,
        normal,
    );
    let face_inverse_mass = linked_soft_body_face_inverse_mass(face_body, face);
    let inverse_mass = cluster_inverse_mass + face_inverse_mass;
    let relative_velocity = frame.linear_velocity + frame.angular_velocity.cross(contact_offset)
        - linked_soft_body_face_velocity(face_body, face);
    let tangent = (relative_velocity - normal * relative_velocity.dot(normal)).normalize_or_zero();
    let tangent_inverse_mass = linked_soft_body_cluster_inverse_mass(
        cluster.linear_impulse_scale,
        frame.inverse_inertia,
        contact_offset,
        tangent,
    ) + face_inverse_mass;
    let impulse = linked_soft_body_project_contact_impulse(
        relative_velocity,
        normal,
        0.5 * (cluster_body.configuration.restitution + face_body.configuration.restitution),
        cluster_body.configuration.friction * face_body.configuration.friction,
        inverse_mass,
        tangent_inverse_mass,
    );
    (impulse, inverse_mass)
}

fn solve_linked_soft_body_cluster_face_contacts(
    cluster_body: &mut LinkedSoftBodyState,
    face_body: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let cluster_vertex_mass =
        cluster_body.configuration.mass / f32_from_usize(cluster_body.vertices.len());
    let mut accumulated_positions = cluster_body
        .configuration
        .accumulate_cluster_corrections
        .then(|| vec![Vec3::ZERO; cluster_body.vertices.len()]);
    for cluster_index in 0..cluster_body.clusters.len() {
        let frame = linked_soft_body_cluster_frame(
            &cluster_body.vertices,
            &cluster_body.previous_vertices,
            &cluster_body.clusters[cluster_index],
            cluster_vertex_mass,
            time_step,
        );
        let radius = frame.radius
            + cluster_body.configuration.collision_radius
            + face_body.configuration.collision_radius;
        let Some((face, weights, distance_squared)) =
            nearest_linked_soft_body_face(face_body, frame.center, radius * radius)
        else {
            continue;
        };
        let triangle = face.map(|index| face_body.vertices[index]);
        let point = triangle[0] * weights[0] + triangle[1] * weights[1] + triangle[2] * weights[2];
        let normal = (frame.center - point).normalize_or_zero();
        if normal == Vec3::ZERO {
            continue;
        }
        let cluster = &cluster_body.clusters[cluster_index];
        let (impulse, inverse_mass) = linked_soft_body_cluster_face_impulse(
            cluster_body,
            face_body,
            cluster,
            frame,
            face,
            point,
            normal,
        );
        let original_positions = capture_linked_soft_body_cluster_positions(
            &cluster_body.vertices,
            cluster,
            accumulated_positions.is_some(),
        );
        linked_soft_body_apply_cluster_velocity_impulse(
            &mut cluster_body.vertices,
            &mut cluster_body.velocities,
            cluster,
            frame,
            point,
            impulse,
            time_step,
        );
        for index in face {
            let delta = impulse * (face_body.inverse_masses[index] / 3.0);
            face_body.velocities[index] -= delta;
            face_body.vertices[index] -= delta * time_step;
        }

        let penetration = radius - distance_squared.sqrt();
        if penetration > 0.0 && inverse_mass > f32::EPSILON {
            let correction_impulse = normal * (penetration / inverse_mass);
            linked_soft_body_apply_cluster_position_impulse(
                &mut cluster_body.vertices,
                cluster,
                frame,
                point,
                correction_impulse,
            );
            for index in face {
                face_body.vertices[index] -=
                    correction_impulse * (face_body.inverse_masses[index] / 3.0);
            }
        }
        defer_linked_soft_body_cluster_positions(
            &mut cluster_body.vertices,
            cluster,
            original_positions,
            accumulated_positions.as_mut(),
        );
    }
    if let Some(accumulated) = accumulated_positions {
        for (vertex, correction) in cluster_body.vertices.iter_mut().zip(accumulated) {
            *vertex += correction;
        }
    }
}

fn solve_linked_soft_body_cluster_vertex_contacts(
    cluster_body: &mut LinkedSoftBodyState,
    vertex_body: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let cluster_vertex_mass =
        cluster_body.configuration.mass / f32_from_usize(cluster_body.vertices.len());
    let radius_padding =
        cluster_body.configuration.collision_radius + vertex_body.configuration.collision_radius;
    let mut accumulated_positions = cluster_body
        .configuration
        .accumulate_cluster_corrections
        .then(|| vec![Vec3::ZERO; cluster_body.vertices.len()]);
    for cluster_index in 0..cluster_body.clusters.len() {
        let frame = linked_soft_body_cluster_frame(
            &cluster_body.vertices,
            &cluster_body.previous_vertices,
            &cluster_body.clusters[cluster_index],
            cluster_vertex_mass,
            time_step,
        );
        for vertex in 0..vertex_body.vertices.len() {
            let delta = frame.center - vertex_body.vertices[vertex];
            let distance = delta.length();
            let radius = frame.radius + radius_padding;
            if distance >= radius || distance <= f32::EPSILON {
                continue;
            }
            let normal = delta / distance;
            let point = vertex_body.vertices[vertex];
            let offset = point - frame.center;
            let cluster = &cluster_body.clusters[cluster_index];
            let cluster_inverse_mass = linked_soft_body_cluster_inverse_mass(
                cluster.linear_impulse_scale,
                frame.inverse_inertia,
                offset,
                normal,
            );
            let vertex_inverse_mass = vertex_body.inverse_masses[vertex];
            let normal_inverse_mass = cluster_inverse_mass + vertex_inverse_mass;
            let cluster_velocity = frame.linear_velocity + frame.angular_velocity.cross(offset);
            let relative_velocity = cluster_velocity - vertex_body.velocities[vertex];
            let tangent =
                (relative_velocity - normal * relative_velocity.dot(normal)).normalize_or_zero();
            let tangent_inverse_mass = linked_soft_body_cluster_inverse_mass(
                cluster.linear_impulse_scale,
                frame.inverse_inertia,
                offset,
                tangent,
            ) + vertex_inverse_mass;
            let impulse = linked_soft_body_project_contact_impulse(
                relative_velocity,
                normal,
                0.5 * (cluster_body.configuration.restitution
                    + vertex_body.configuration.restitution),
                cluster_body.configuration.friction * vertex_body.configuration.friction,
                normal_inverse_mass,
                tangent_inverse_mass,
            );
            let original_positions = capture_linked_soft_body_cluster_positions(
                &cluster_body.vertices,
                cluster,
                accumulated_positions.is_some(),
            );
            linked_soft_body_apply_cluster_velocity_impulse(
                &mut cluster_body.vertices,
                &mut cluster_body.velocities,
                cluster,
                frame,
                point,
                impulse,
                time_step,
            );
            vertex_body.velocities[vertex] -= impulse * vertex_inverse_mass;
            vertex_body.vertices[vertex] -= impulse * vertex_inverse_mass * time_step;
            if normal_inverse_mass > f32::EPSILON {
                let correction = normal * ((radius - distance) / normal_inverse_mass);
                linked_soft_body_apply_cluster_position_impulse(
                    &mut cluster_body.vertices,
                    cluster,
                    frame,
                    point,
                    correction,
                );
                vertex_body.vertices[vertex] -= correction * vertex_inverse_mass;
            }
            defer_linked_soft_body_cluster_positions(
                &mut cluster_body.vertices,
                cluster,
                original_positions,
                accumulated_positions.as_mut(),
            );
        }
    }
    if let Some(accumulated) = accumulated_positions {
        for (vertex, correction) in cluster_body.vertices.iter_mut().zip(accumulated) {
            *vertex += correction;
        }
    }
}

/// Closest witness pair between two triangles: the point on `left`, the point
/// on `right`, and their squared distance.
fn closest_face_pair(left: [Vec3; 3], right: [Vec3; 3]) -> (Vec3, Vec3, f32) {
    let mut closest = (Vec3::ZERO, Vec3::ZERO, f32::MAX);
    for &point in &left {
        let (weights, distance_squared) = closest_triangle_weights(point, right);
        if distance_squared < closest.2 {
            closest = (
                point,
                right[0] * weights[0] + right[1] * weights[1] + right[2] * weights[2],
                distance_squared,
            );
        }
    }
    for &point in &right {
        let (weights, distance_squared) = closest_triangle_weights(point, left);
        if distance_squared < closest.2 {
            closest = (
                left[0] * weights[0] + left[1] * weights[1] + left[2] * weights[2],
                point,
                distance_squared,
            );
        }
    }
    closest
}

fn solve_linked_soft_body_face_contacts(
    left: &mut LinkedSoftBodyState,
    right: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let radius = left.configuration.collision_radius + right.configuration.collision_radius;
    let radius_squared = radius * radius;
    for &left_face in &left.configuration.faces {
        let left_indices = left_face.map(|index| index as usize);
        let left_triangle = left_indices.map(|index| left.vertices[index]);
        for &right_face in &right.configuration.faces {
            let right_indices = right_face.map(|index| index as usize);
            let right_triangle = right_indices.map(|index| right.vertices[index]);
            let closest = closest_face_pair(left_triangle, right_triangle);
            if closest.2 >= radius_squared {
                continue;
            }
            let relative_velocity = linked_soft_body_face_velocity(left, left_indices)
                - linked_soft_body_face_velocity(right, right_indices);
            let separation = closest.0 - closest.1;
            let normal = if separation.length_squared() > f32::EPSILON {
                separation.normalize()
            } else {
                let face_normal = (left_triangle[1] - left_triangle[0])
                    .cross(left_triangle[2] - left_triangle[0])
                    .normalize_or_zero();
                if relative_velocity.dot(face_normal) <= 0.0 {
                    face_normal
                } else {
                    -face_normal
                }
            };
            if normal == Vec3::ZERO {
                continue;
            }
            let inverse_mass = linked_soft_body_face_inverse_mass(left, left_indices)
                + linked_soft_body_face_inverse_mass(right, right_indices);
            let impulse = linked_soft_body_project_contact_impulse(
                relative_velocity,
                normal,
                0.5 * (left.configuration.restitution + right.configuration.restitution),
                left.configuration.friction * right.configuration.friction,
                inverse_mass,
                inverse_mass,
            );
            for index in left_indices {
                let delta = impulse * (left.inverse_masses[index] / 3.0);
                left.velocities[index] += delta;
                left.vertices[index] += delta * time_step;
            }
            for index in right_indices {
                let delta = impulse * (right.inverse_masses[index] / 3.0);
                right.velocities[index] -= delta;
                right.vertices[index] -= delta * time_step;
            }
            if inverse_mass > f32::EPSILON {
                let correction = normal * ((radius - closest.2.sqrt()) / inverse_mass);
                for index in left_indices {
                    left.vertices[index] += correction * (left.inverse_masses[index] / 3.0);
                }
                for index in right_indices {
                    right.vertices[index] -= correction * (right.inverse_masses[index] / 3.0);
                }
            }
        }
    }
}

/// One deformable's aggregate cluster as a contact feature: its body, the
/// cluster's membership, and the frame fitted to it this substep.
type ClusterFeature<'a> = (
    &'a LinkedSoftBodyState,
    &'a LinkedSoftBodyCluster,
    LinkedSoftBodyClusterFrame,
);

/// Impulse between two aggregate clusters, together with the normal-direction
/// inverse mass the position pass reuses.
fn linked_soft_body_cluster_pair_impulse(
    left: ClusterFeature<'_>,
    right: ClusterFeature<'_>,
    point: Vec3,
    normal: Vec3,
) -> (Vec3, f32) {
    let (left_body, left_cluster, left_frame) = left;
    let (right_body, right_cluster, right_frame) = right;
    let left_offset = point - left_frame.center;
    let right_offset = point - right_frame.center;
    let pair_inverse_mass = |direction: Vec3| {
        linked_soft_body_cluster_inverse_mass(
            left_cluster.linear_impulse_scale,
            left_frame.inverse_inertia,
            left_offset,
            direction,
        ) + linked_soft_body_cluster_inverse_mass(
            right_cluster.linear_impulse_scale,
            right_frame.inverse_inertia,
            right_offset,
            direction,
        )
    };
    let normal_inverse_mass = pair_inverse_mass(normal);
    let relative_velocity = left_frame.linear_velocity
        + left_frame.angular_velocity.cross(left_offset)
        - right_frame.linear_velocity
        - right_frame.angular_velocity.cross(right_offset);
    let tangent = (relative_velocity - normal * relative_velocity.dot(normal)).normalize_or_zero();
    let impulse = linked_soft_body_project_contact_impulse(
        relative_velocity,
        normal,
        0.5 * (left_body.configuration.restitution + right_body.configuration.restitution),
        left_body.configuration.friction * right_body.configuration.friction,
        normal_inverse_mass,
        pair_inverse_mass(tangent),
    );
    (impulse, normal_inverse_mass)
}

/// One side of a cluster-to-cluster contact: the body, which of its clusters is
/// in contact, that cluster's fitted frame, and the deferred position
/// accumulator when the body accumulates corrections.
type ClusterSide<'a> = (
    &'a mut LinkedSoftBodyState,
    usize,
    LinkedSoftBodyClusterFrame,
    Option<&'a mut Vec<Vec3>>,
);

/// Applies a velocity impulse and an optional position correction to one
/// cluster, deferring the position change when the body accumulates corrections
/// across clusters.
fn apply_cluster_pair_impulse(
    side: ClusterSide<'_>,
    point: Vec3,
    impulse: Vec3,
    correction: Option<Vec3>,
    time_step: f32,
) {
    let (body, cluster_index, frame, accumulated) = side;
    let cluster = &body.clusters[cluster_index];
    let original =
        capture_linked_soft_body_cluster_positions(&body.vertices, cluster, accumulated.is_some());
    linked_soft_body_apply_cluster_velocity_impulse(
        &mut body.vertices,
        &mut body.velocities,
        cluster,
        frame,
        point,
        impulse,
        time_step,
    );
    if let Some(correction) = correction {
        linked_soft_body_apply_cluster_position_impulse(
            &mut body.vertices,
            cluster,
            frame,
            point,
            correction,
        );
    }
    defer_linked_soft_body_cluster_positions(&mut body.vertices, cluster, original, accumulated);
}

fn solve_linked_soft_body_cluster_contacts(
    left: &mut LinkedSoftBodyState,
    right: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let left_vertex_mass = left.configuration.mass / f32_from_usize(left.vertices.len());
    let right_vertex_mass = right.configuration.mass / f32_from_usize(right.vertices.len());
    let mut left_accumulated = left
        .configuration
        .accumulate_cluster_corrections
        .then(|| vec![Vec3::ZERO; left.vertices.len()]);
    let mut right_accumulated = right
        .configuration
        .accumulate_cluster_corrections
        .then(|| vec![Vec3::ZERO; right.vertices.len()]);
    for left_index in 0..left.clusters.len() {
        for right_index in 0..right.clusters.len() {
            let left_frame = linked_soft_body_cluster_frame(
                &left.vertices,
                &left.previous_vertices,
                &left.clusters[left_index],
                left_vertex_mass,
                time_step,
            );
            let right_frame = linked_soft_body_cluster_frame(
                &right.vertices,
                &right.previous_vertices,
                &right.clusters[right_index],
                right_vertex_mass,
                time_step,
            );
            let radius = left_frame.radius
                + right_frame.radius
                + left.configuration.collision_radius
                + right.configuration.collision_radius;
            let center_delta = left_frame.center - right_frame.center;
            let distance = center_delta.length();
            if distance >= radius {
                continue;
            }
            let normal = center_delta.normalize_or_zero();
            if normal == Vec3::ZERO {
                continue;
            }
            let point = left_frame.center - normal * left_frame.radius;
            let (impulse, normal_inverse_mass) = linked_soft_body_cluster_pair_impulse(
                (&*left, &left.clusters[left_index], left_frame),
                (&*right, &right.clusters[right_index], right_frame),
                point,
                normal,
            );
            let correction = (normal_inverse_mass > f32::EPSILON)
                .then(|| normal * ((radius - distance) / normal_inverse_mass));
            apply_cluster_pair_impulse(
                (
                    &mut *left,
                    left_index,
                    left_frame,
                    left_accumulated.as_mut(),
                ),
                point,
                impulse,
                correction,
                time_step,
            );
            apply_cluster_pair_impulse(
                (
                    &mut *right,
                    right_index,
                    right_frame,
                    right_accumulated.as_mut(),
                ),
                point,
                -impulse,
                correction.map(|correction| -correction),
                time_step,
            );
        }
    }
    if let Some(accumulated) = left_accumulated {
        for (vertex, correction) in left.vertices.iter_mut().zip(accumulated) {
            *vertex += correction;
        }
    }
    if let Some(accumulated) = right_accumulated {
        for (vertex, correction) in right.vertices.iter_mut().zip(accumulated) {
            *vertex += correction;
        }
    }
}

fn solve_linked_soft_body_vertex_contacts(
    left: &mut LinkedSoftBodyState,
    right: &mut LinkedSoftBodyState,
    time_step: f32,
) {
    let radius = left.configuration.collision_radius + right.configuration.collision_radius;
    for left_index in 0..left.vertices.len() {
        for right_index in 0..right.vertices.len() {
            let delta = left.vertices[left_index] - right.vertices[right_index];
            let distance = delta.length();
            if distance >= radius || distance <= f32::EPSILON {
                continue;
            }
            let normal = delta / distance;
            let left_inverse_mass = left.inverse_masses[left_index];
            let right_inverse_mass = right.inverse_masses[right_index];
            let inverse_mass = left_inverse_mass + right_inverse_mass;
            let impulse = linked_soft_body_project_contact_impulse(
                left.velocities[left_index] - right.velocities[right_index],
                normal,
                0.5 * (left.configuration.restitution + right.configuration.restitution),
                left.configuration.friction * right.configuration.friction,
                inverse_mass,
                inverse_mass,
            );
            left.velocities[left_index] += impulse * left_inverse_mass;
            right.velocities[right_index] -= impulse * right_inverse_mass;
            left.vertices[left_index] += impulse * left_inverse_mass * time_step;
            right.vertices[right_index] -= impulse * right_inverse_mass * time_step;
            if inverse_mass > f32::EPSILON {
                let correction = normal * ((radius - distance) / inverse_mass);
                left.vertices[left_index] += correction * left_inverse_mass;
                right.vertices[right_index] -= correction * right_inverse_mass;
            }
        }
    }
}

fn linked_soft_body_project_contact_impulse(
    relative_velocity: Vec3,
    normal: Vec3,
    restitution: f32,
    friction: f32,
    normal_inverse_mass: f32,
    tangent_inverse_mass: f32,
) -> Vec3 {
    if normal_inverse_mass <= f32::EPSILON {
        return Vec3::ZERO;
    }
    let normal_speed = relative_velocity.dot(normal);
    let normal_impulse = if normal_speed < 0.0 {
        -(1.0 + restitution) * normal_speed / normal_inverse_mass
    } else {
        0.0
    };
    let tangent_velocity = relative_velocity - normal * normal_speed;
    let tangent = tangent_velocity.normalize_or_zero();
    let tangent_impulse = if tangent != Vec3::ZERO && tangent_inverse_mass > f32::EPSILON {
        (-tangent_velocity.length() / tangent_inverse_mass).max(-friction * normal_impulse)
    } else {
        0.0
    };
    normal * normal_impulse + tangent * tangent_impulse
}

fn capture_linked_soft_body_cluster_positions(
    vertices: &[Vec3],
    cluster: &LinkedSoftBodyCluster,
    enabled: bool,
) -> Option<SmallVec<[Vec3; 16]>> {
    enabled.then(|| {
        cluster
            .vertices
            .iter()
            .map(|&vertex| vertices[vertex])
            .collect()
    })
}

fn defer_linked_soft_body_cluster_positions(
    vertices: &mut [Vec3],
    cluster: &LinkedSoftBodyCluster,
    original: Option<SmallVec<[Vec3; 16]>>,
    accumulated: Option<&mut Vec<Vec3>>,
) {
    let (Some(original), Some(accumulated)) = (original, accumulated) else {
        return;
    };
    for (&vertex, original) in cluster.vertices.iter().zip(original) {
        accumulated[vertex] += vertices[vertex] - original;
        vertices[vertex] = original;
    }
}

#[inline]
fn linked_soft_body_random_range(state: &mut u64, minimum: f32, maximum: f32) -> f32 {
    const MULTIPLIER: u64 = 0x0005_deec_e66d;
    const ADDEND: u64 = 0xb;
    const MASK: u64 = (1_u64 << 48) - 1;
    *state = state.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND) & MASK;
    // `state` is masked to 48 bits, so the shifted value always fits a `u32`.
    let sample = u32::try_from(*state >> 16).unwrap_or(u32::MAX) % 1001;
    (maximum - minimum).mul_add(f32_from_u32(sample) * 0.001, minimum)
}

fn linked_soft_body_cluster_frame(
    vertices: &[Vec3],
    previous_vertices: &[Vec3],
    cluster: &LinkedSoftBodyCluster,
    vertex_mass: f32,
    time_step: f32,
) -> LinkedSoftBodyClusterFrame {
    let reciprocal_count = 1.0 / f32_from_usize(cluster.vertices.len());
    let center = cluster
        .vertices
        .iter()
        .map(|&vertex| vertices[vertex])
        .sum::<Vec3>()
        * reciprocal_count;
    let previous_center = cluster
        .vertices
        .iter()
        .map(|&vertex| previous_vertices[vertex])
        .sum::<Vec3>()
        * reciprocal_count;
    let linear_velocity = (center - previous_center) / time_step;
    let mut inertia = Mat3::ZERO;
    let mut angular_momentum = Vec3::ZERO;
    let mut radius_squared = 0.0_f32;
    for &vertex in &cluster.vertices {
        let offset = vertices[vertex] - center;
        let velocity = (vertices[vertex] - previous_vertices[vertex]) / time_step;
        let outer = Mat3::from_cols(offset * offset.x, offset * offset.y, offset * offset.z);
        inertia += (Mat3::IDENTITY * offset.length_squared() - outer) * vertex_mass;
        angular_momentum += offset.cross(velocity * vertex_mass);
        radius_squared = radius_squared.max(offset.length_squared());
    }
    let inverse_inertia = if inertia.determinant().abs() > 1.0e-10 {
        inertia.inverse()
    } else {
        Mat3::ZERO
    };
    LinkedSoftBodyClusterFrame {
        center,
        previous_center,
        inverse_inertia,
        linear_velocity,
        angular_velocity: inverse_inertia * angular_momentum,
        radius: radius_squared.sqrt(),
    }
}

fn linked_soft_body_cluster_inverse_mass(
    linear_impulse_scale: f32,
    inverse_inertia: Mat3,
    contact_offset: Vec3,
    direction: Vec3,
) -> f32 {
    if direction == Vec3::ZERO {
        return 0.0;
    }
    linear_impulse_scale
        + direction.dot((inverse_inertia * contact_offset.cross(direction)).cross(contact_offset))
}

/// Normal-plus-friction impulse for one aggregate cluster against a rigid
/// contact, together with the normal-direction inverse mass that the position
/// correction reuses.
fn linked_soft_body_cluster_rigid_impulse(
    configuration: &LinkedSoftBodyConfiguration,
    cluster: &LinkedSoftBodyCluster,
    frame: LinkedSoftBodyClusterFrame,
    contact: DeformableContact,
    normal: Vec3,
) -> (Vec3, f32) {
    let contact_offset = contact.position - frame.center;
    let relative_velocity =
        frame.linear_velocity + frame.angular_velocity.cross(contact_offset) - contact.velocity;
    let normal_inverse_mass = linked_soft_body_cluster_inverse_mass(
        cluster.linear_impulse_scale,
        frame.inverse_inertia,
        contact_offset,
        normal,
    );
    let normal_speed = relative_velocity.dot(normal);
    let normal_impulse = if normal_speed < 0.0 && normal_inverse_mass > f32::EPSILON {
        -(1.0 + configuration.restitution) * normal_speed / normal_inverse_mass
    } else {
        0.0
    };
    let tangent_velocity = relative_velocity - normal * normal_speed;
    let tangent = tangent_velocity.normalize_or_zero();
    let tangent_inverse_mass = linked_soft_body_cluster_inverse_mass(
        cluster.linear_impulse_scale,
        frame.inverse_inertia,
        contact_offset,
        tangent,
    );
    let tangent_impulse = if tangent != Vec3::ZERO && tangent_inverse_mass > f32::EPSILON {
        (-tangent_velocity.length() / tangent_inverse_mass)
            .max(-configuration.friction * normal_impulse)
    } else {
        0.0
    };
    (
        normal * normal_impulse + tangent * tangent_impulse,
        normal_inverse_mass,
    )
}

fn linked_soft_body_apply_cluster_velocity_impulse(
    vertices: &mut [Vec3],
    velocities: &mut [Vec3],
    cluster: &LinkedSoftBodyCluster,
    frame: LinkedSoftBodyClusterFrame,
    point: Vec3,
    impulse: Vec3,
    time_step: f32,
) {
    if impulse == Vec3::ZERO {
        return;
    }
    let linear_delta = impulse * cluster.linear_impulse_scale;
    let angular_delta = frame.inverse_inertia * (point - frame.center).cross(impulse);
    for &vertex in &cluster.vertices {
        let delta = linear_delta + angular_delta.cross(vertices[vertex] - frame.center);
        velocities[vertex] += delta;
        vertices[vertex] += delta * time_step;
    }
}

fn linked_soft_body_apply_cluster_position_impulse(
    vertices: &mut [Vec3],
    cluster: &LinkedSoftBodyCluster,
    frame: LinkedSoftBodyClusterFrame,
    point: Vec3,
    impulse: Vec3,
) {
    let linear_delta = impulse * cluster.linear_impulse_scale;
    let angular_delta = frame.inverse_inertia * (point - frame.center).cross(impulse);
    for &vertex in &cluster.vertices {
        let delta = linear_delta + angular_delta.cross(vertices[vertex] - frame.center);
        vertices[vertex] += delta;
    }
}

/// Projects one native linked-soft-body link family.
///
/// The solver invokes this rational squared-length projection once for each
/// link family. Keeping the family slice and coefficient explicit prevents the
/// two native products from being merged during cooking.
fn solve_link_family(
    vertices: &mut [Vec3],
    inverse_masses: &[f32],
    links: &[LinkedSoftBodyLink],
    stiffness: f32,
    minimum_squared_ratio: f32,
) {
    for link in links {
        let [a, b] = link.vertices;
        let delta = vertices[b] - vertices[a];
        let current_length_squared = delta.length_squared();
        if current_length_squared <= f32::EPSILON {
            continue;
        }
        let target_length_squared = current_length_squared
            .min(link.rest_length_squared)
            .max(link.rest_length_squared * minimum_squared_ratio);
        let denominator = current_length_squared + target_length_squared;
        let correction_factor = (target_length_squared - current_length_squared) * stiffness
            / denominator.max(f32::EPSILON);
        let inverse_mass_sum = inverse_masses[a] + inverse_masses[b];
        if inverse_mass_sum <= f32::EPSILON {
            continue;
        }
        let correction = delta * correction_factor;
        vertices[a] -= correction * (inverse_masses[a] / inverse_mass_sum);
        vertices[b] += correction * (inverse_masses[b] / inverse_mass_sum);
    }
}

#[cfg(test)]
mod linked_soft_body_link_tests {
    use super::*;

    #[test]
    fn link_family_coefficient_controls_its_projection() {
        let link = LinkedSoftBodyLink {
            vertices: [0, 1],
            rest_length_squared: 1.0,
        };
        let inverse_masses = [1.0, 1.0];
        let mut disabled = [Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)];
        solve_link_family(&mut disabled, &inverse_masses, &[link], 0.0, 0.0);
        assert!(
            (disabled[1].x - 2.0).abs() <= f32::EPSILON,
            "a zero-stiffness family must leave its vertices untouched"
        );

        let mut enabled = [Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0)];
        solve_link_family(&mut enabled, &inverse_masses, &[link], 1.0, 0.0);
        assert!(enabled[1].x - enabled[0].x < 2.0);
    }

    #[test]
    fn cluster_impulse_moves_every_member_through_one_aggregate_frame() {
        let cluster = LinkedSoftBodyCluster {
            vertices: vec![0, 1].into_boxed_slice(),
            linear_impulse_scale: 0.5,
        };
        let mut vertices = vec![Vec3::ZERO, Vec3::Y];
        let previous = vertices.clone();
        let mut velocities = vec![Vec3::ZERO; 2];
        let frame = linked_soft_body_cluster_frame(&vertices, &previous, &cluster, 0.5, 1.0);
        linked_soft_body_apply_cluster_velocity_impulse(
            &mut vertices,
            &mut velocities,
            &cluster,
            frame,
            frame.center,
            Vec3::X,
            1.0,
        );
        assert_eq!(velocities, vec![Vec3::new(0.5, 0.0, 0.0); 2]);
        assert_eq!(
            vertices,
            vec![Vec3::new(0.5, 0.0, 0.0), Vec3::new(0.5, 1.0, 0.0)]
        );
    }

    #[test]
    fn pose_fit_recovers_planar_rotation_and_translation() {
        let rest = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::X + Vec3::Y];
        let expected = PhysicsPose {
            translation: Vec3::new(3.0, -2.0, 7.0),
            rotation: Quat::from_rotation_z(core::f32::consts::FRAC_PI_2),
        };
        let world = rest.map(|vertex| expected.transform_point(vertex));
        let fitted = fit_linked_soft_body_pose(&rest, &world, Quat::IDENTITY).unwrap();

        assert!(fitted.translation.abs_diff_eq(expected.translation, 1.0e-4));
        for (&source, &target) in rest.iter().zip(&world) {
            assert!(fitted.transform_point(source).abs_diff_eq(target, 1.0e-4));
        }
    }
}

fn calculate_vertex_normals(vertices: &[Vec3], faces: &[[u32; 3]], output: &mut [Vec3]) {
    output.fill(Vec3::ZERO);
    for &face in faces {
        let [a, b, c] = face.map(|index| index as usize);
        let normal = (vertices[b] - vertices[a]).cross(vertices[c] - vertices[a]);
        output[a] += normal;
        output[b] += normal;
        output[c] += normal;
    }
    for normal in output {
        *normal = normal.normalize_or_zero();
    }
}

fn linked_soft_body_volume(vertices: &[Vec3], faces: &[[u32; 3]]) -> f32 {
    faces
        .iter()
        .map(|face| {
            let [a, b, c] = face.map(|index| vertices[index as usize]);
            a.dot(b.cross(c)) / 6.0
        })
        .sum()
}

fn lift_drag_force(
    mut normal: Vec3,
    relative_velocity: Vec3,
    area: f32,
    density: f32,
    lift: f32,
    drag: f32,
    one_sided: bool,
) -> Vec3 {
    let speed_squared = relative_velocity.length_squared();
    if speed_squared <= f32::EPSILON {
        return Vec3::ZERO;
    }
    let speed = speed_squared.sqrt();
    let direction = relative_velocity / speed;
    let mut incidence = normal.dot(direction);
    if one_sided {
        if incidence <= 0.0 {
            return Vec3::ZERO;
        }
    } else if incidence < 0.0 {
        normal = -normal;
        incidence = -incidence;
    }
    let dynamic_pressure = 0.5 * density * area * speed_squared * incidence;
    let drag_force = -direction * (dynamic_pressure * drag);
    let lift_direction = normal.cross(direction).cross(direction).normalize_or_zero();
    let lift_force = lift_direction
        * (0.5
            * lift
            * density
            * speed
            * area
            * incidence.mul_add(-incidence, 1.0).max(0.0).sqrt());
    drag_force + lift_force
}

#[derive(Debug, Clone, Copy)]
pub struct DeformableContact {
    pub position: Vec3,
    pub normal: Vec3,
    /// Distance travelled by the query shape before contact. Zero denotes an
    /// overlap at the query's starting pose.
    pub distance: f32,
    pub velocity: Vec3,
    pub body: PhysicsBodyHandle,
    pub dynamic: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct DeformableReaction {
    pub body: PhysicsBodyHandle,
    pub point: Vec3,
    pub impulse: Vec3,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MediumSample {
    pub submerged_fraction: f32,
    pub submerged_depth: f32,
    pub water_density: f32,
    pub velocity: Vec3,
    pub gravity: Option<Vec3>,
}

// `CPhysicalWorld` initializes `accuracyMC` to this value. The rope
// multicontact solver consumes the world value directly in CryPhysics.
const CRY_MULTICONTACT_ACCURACY: f32 = 0.002;

#[derive(Debug, Clone, Copy)]
struct RopeSolverVertex {
    position: Vec3,
    velocity: Vec3,
    source_segment: usize,
    main_vertex: Option<usize>,
    contact: Option<DeformableContact>,
    friction_impulse: f32,
    contact_velocity_delta: Vec3,
}

impl RopeSolverVertex {
    const fn main(
        position: Vec3,
        velocity: Vec3,
        source_segment: usize,
        main_vertex: usize,
        contact: Option<DeformableContact>,
    ) -> Self {
        Self {
            position,
            velocity,
            source_segment,
            main_vertex: Some(main_vertex),
            contact,
            friction_impulse: 0.0,
            contact_velocity_delta: Vec3::ZERO,
        }
    }

    const fn contact(
        position: Vec3,
        velocity: Vec3,
        source_segment: usize,
        contact: DeformableContact,
    ) -> Self {
        Self {
            position,
            velocity,
            source_segment,
            main_vertex: None,
            contact: Some(contact),
            friction_impulse: 0.0,
            contact_velocity_delta: Vec3::ZERO,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "these are independent Cry rope conditions republished verbatim in `RopeStatus`, not one state enum"
)]
pub struct RopeState {
    pub configuration: RopeBodyConfiguration,
    pub query_collider: ColliderHandle,
    pub points: Vec<Vec3>,
    pub previous_points: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub contact_normals: Vec<Vec3>,
    pub contact_bodies: Vec<Option<PhysicsBodyHandle>>,
    pub contact_velocities: Vec<Vec3>,
    contact_points: Vec<Vec3>,
    contact_dynamic: Vec<bool>,
    pub target_points: Vec<Vec3>,
    target_initialized: bool,
    pub target_mode: RopeTargetPoseMode,
    pub host_position: Vec3,
    pub host_rotation: Quat,
    pub static_contacts: u32,
    pub dynamic_contacts: u32,
    pub strained: bool,
    pub torn: bool,
    pub awake: bool,
    pub time_last_active: f32,
    pub entity_pose: PhysicsPose,
    rest_lengths: Vec<f32>,
    inverse_masses: Vec<f32>,
    subdivision_vertices: Vec<Vec3>,
    pending_force: Vec3,
    pending_torque: Vec3,
    reactions: SmallVec<[DeformableReaction; 16]>,
}

impl RopeState {
    pub(crate) fn new(
        configuration: RopeBodyConfiguration,
        pose: PhysicsPose,
        query_collider: ColliderHandle,
    ) -> Self {
        let points: Vec<_> = configuration
            .points
            .iter()
            .map(|point| pose.transform_point(*point))
            .collect();
        let velocities = if configuration.velocities.is_empty() {
            vec![Vec3::ZERO; points.len()]
        } else {
            configuration
                .velocities
                .iter()
                .map(|velocity| pose.rotation * *velocity)
                .collect()
        };
        let segment_count = points.len() - 1;
        let rest_lengths = if configuration.target_length > 0.0 {
            vec![configuration.target_length / f32_from_usize(segment_count); segment_count]
        } else {
            points
                .windows(2)
                .map(|points| points[0].distance(points[1]))
                .collect()
        };
        let inverse_vertex_mass = f32_from_usize(points.len()) / configuration.mass;
        let mut inverse_masses = vec![inverse_vertex_mass; points.len()];
        if configuration.attachments[0].is_some() {
            inverse_masses[0] = 0.0;
        }
        if configuration.attachments[1].is_some() {
            inverse_masses[points.len() - 1] = 0.0;
        }
        let target_initialized = !configuration.target_points.is_empty();
        let target_points = if configuration.target_points.is_empty() {
            vec![Vec3::ZERO; points.len()]
        } else if configuration
            .flags
            .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_START)
            || configuration
                .flags
                .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_END)
        {
            configuration.target_points.clone()
        } else {
            configuration
                .target_points
                .iter()
                .map(|point| pose.transform_point(*point))
                .collect()
        };
        let target_mode = configuration.target_pose_mode;
        Self {
            configuration,
            query_collider,
            previous_points: points.clone(),
            velocities,
            contact_normals: vec![Vec3::ZERO; points.len()],
            contact_bodies: vec![None; points.len()],
            contact_velocities: vec![Vec3::ZERO; points.len()],
            contact_points: vec![Vec3::ZERO; points.len()],
            contact_dynamic: vec![false; points.len()],
            points,
            target_points,
            target_initialized,
            target_mode,
            host_position: pose.translation,
            host_rotation: pose.rotation,
            static_contacts: 0,
            dynamic_contacts: 0,
            strained: false,
            torn: false,
            awake: true,
            time_last_active: 0.0,
            entity_pose: pose,
            rest_lengths,
            inverse_masses,
            subdivision_vertices: Vec::new(),
            pending_force: Vec3::ZERO,
            pending_torque: Vec3::ZERO,
            reactions: SmallVec::new(),
        }
    }

    pub(crate) fn set_pose(&mut self, pose: PhysicsPose) {
        let delta = pose * self.entity_pose.inverse();
        for point in &mut self.points {
            *point = delta.transform_point(*point);
        }
        for point in &mut self.previous_points {
            *point = delta.transform_point(*point);
        }
        for point in &mut self.subdivision_vertices {
            *point = delta.transform_point(*point);
        }
        if self.target_initialized
            && !self
                .configuration
                .flags
                .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_START)
            && !self
                .configuration
                .flags
                .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_END)
        {
            for point in &mut self.target_points {
                *point = delta.transform_point(*point);
            }
        }
        for velocity in &mut self.velocities {
            *velocity = delta.rotation * *velocity;
        }
        self.entity_pose = pose;
        self.awake = true;
    }

    pub(crate) fn reset(&mut self) {
        for (point, source) in self.points.iter_mut().zip(&self.configuration.points) {
            *point = self.entity_pose.transform_point(*source);
        }
        self.previous_points.clone_from(&self.points);
        if self.configuration.velocities.is_empty() {
            self.velocities.fill(Vec3::ZERO);
        } else {
            for (velocity, source) in self
                .velocities
                .iter_mut()
                .zip(&self.configuration.velocities)
            {
                *velocity = self.entity_pose.rotation * *source;
            }
        }
        self.contact_normals.fill(Vec3::ZERO);
        self.contact_bodies.fill(None);
        self.contact_velocities.fill(Vec3::ZERO);
        self.contact_points.fill(Vec3::ZERO);
        self.contact_dynamic.fill(false);
        self.subdivision_vertices.clear();
        self.static_contacts = 0;
        self.dynamic_contacts = 0;
        self.strained = false;
        self.torn = false;
        self.pending_force = Vec3::ZERO;
        self.pending_torque = Vec3::ZERO;
        self.reactions.clear();
        self.awake = true;
    }

    pub(crate) fn set_velocity(&mut self, velocity: Vec3) {
        for (value, inverse_mass) in self.velocities.iter_mut().zip(&self.inverse_masses) {
            if *inverse_mass > 0.0 {
                *value = velocity;
            }
        }
        self.awake = true;
    }

    pub(crate) fn apply_impulse(&mut self, action: ImpulseAction) {
        if let Some(point) = action.point {
            let (segment, fraction, distance_squared) = nearest_segment(&self.points, point);
            let collision_distance = self.segment_thickness(segment);
            if distance_squared > (collision_distance * 3.0).powi(2) {
                return;
            }
            let inverse_vertex_mass = f32_from_usize(self.points.len()) / self.configuration.mass;
            if self.inverse_masses[segment] > 0.0 {
                self.velocities[segment] +=
                    action.impulse * (inverse_vertex_mass * (1.0 - fraction));
            }
            if self.inverse_masses[segment + 1] > 0.0 {
                self.velocities[segment + 1] += action.impulse * (inverse_vertex_mass * fraction);
            }
        } else {
            let index = (self.points.len() - 1) / 2;
            if self.inverse_masses[index] > 0.0 {
                self.velocities[index] += action.impulse * self.inverse_masses[index];
            }
        }
        self.awake = true;
    }

    pub(crate) fn apply_angular_impulse(&mut self, impulse: Vec3) {
        let (_, radius) = deformable_bounds(&self.points);
        let inertia = (self.configuration.mass * radius * radius).max(1.0e-6);
        self.add_angular_velocity(impulse / inertia);
    }

    pub(crate) fn set_angular_velocity(&mut self, angular_velocity: Vec3) {
        let (center, _) = deformable_bounds(&self.points);
        for ((velocity, point), inverse_mass) in self
            .velocities
            .iter_mut()
            .zip(&self.points)
            .zip(&self.inverse_masses)
        {
            if *inverse_mass > 0.0 {
                *velocity = angular_velocity.cross(*point - center);
            }
        }
        self.awake = true;
    }

    fn add_angular_velocity(&mut self, angular_velocity: Vec3) {
        let (center, _) = deformable_bounds(&self.points);
        for ((velocity, point), inverse_mass) in self
            .velocities
            .iter_mut()
            .zip(&self.points)
            .zip(&self.inverse_masses)
        {
            if *inverse_mass > 0.0 {
                *velocity += angular_velocity.cross(*point - center);
            }
        }
        self.awake = true;
    }

    pub(crate) fn add_force(&mut self, force: Vec3) {
        self.pending_force += force;
        self.awake = true;
    }

    pub(crate) fn add_torque(&mut self, torque: Vec3) {
        self.pending_torque += torque;
        self.awake = true;
    }

    pub(crate) fn set_mass(&mut self, mass: f32) {
        self.configuration.mass = mass;
        let point_count = self.points.len();
        let inverse_mass = f32_from_usize(point_count) / mass;
        let start_attached = self.configuration.attachments[0].is_some();
        let end_attached = self.configuration.attachments[1].is_some();
        for (index, value) in self.inverse_masses.iter_mut().enumerate() {
            let attached =
                (index == 0 && start_attached) || (index == point_count - 1 && end_attached);
            *value = if attached { 0.0 } else { inverse_mass };
        }
    }

    pub(crate) fn set_target(
        &mut self,
        action: &DeformableTargetVertices,
        relative_frame: Option<AttachmentFrame>,
    ) -> Result<(), PhysicsError> {
        action.validate()?;
        if let Some(points) = &action.points {
            if points.len() != self.points.len() {
                return Err(PhysicsError::InvalidRopeConfiguration {
                    field: "target vertex count",
                });
            }
            self.target_points.clone_from(points);
        } else if let Some(frame) = relative_frame {
            let inverse_rotation = frame.rotation.inverse();
            for (target, point) in self.target_points.iter_mut().zip(&self.points) {
                *target = inverse_rotation * (*point - frame.position);
            }
        } else {
            self.target_points.clone_from(&self.points);
        }
        if let Some(frame) = relative_frame {
            self.host_position = frame.position;
            self.host_rotation = frame.rotation;
        } else {
            self.host_position = Vec3::ZERO;
            self.host_rotation = Quat::IDENTITY;
        }
        self.target_initialized = true;
        self.awake = true;
        Ok(())
    }

    pub(crate) const fn notify_attachment_moved(&mut self) {
        self.awake = true;
    }

    pub(crate) fn apply_volumetric_pressure(
        &mut self,
        pressure: RopeVolumetricPressure,
    ) -> Result<(), PhysicsError> {
        pressure.validate()?;
        if self.configuration.maximum_force == 0.0
            || !self.configuration.attachments.iter().all(Option::is_some)
        {
            self.awake = true;
            return Ok(());
        }

        let rope_length = self.configuration.target_length;
        let mut exposure = 0.0;
        let mut average_thickness = 0.0;
        for segment in 0..self.rest_lengths.len() {
            let radius =
                (self.points[segment] + self.points[segment + 1]) * 0.5 - pressure.epicenter;
            let radius_length = radius.length();
            if radius_length > f32::EPSILON {
                let direction =
                    (self.points[segment + 1] - self.points[segment]).normalize_or_zero();
                exposure += direction.cross(radius).length()
                    / (radius_length
                        * radius
                            .length_squared()
                            .max(pressure.minimum_radius * pressure.minimum_radius));
            }
            average_thickness += self.segment_thickness(segment);
        }
        average_thickness /= f32_from_usize(self.rest_lengths.len());

        let load = exposure * pressure.pressure_scale * rope_length * average_thickness * 2.0;
        if load > self.configuration.maximum_force * 0.01 {
            let start_distance = self.points[0].distance_squared(pressure.epicenter);
            let end_index = self.points.len() - 1;
            let end_distance = self.points[end_index].distance_squared(pressure.epicenter);
            let side = usize::from(end_distance < start_distance);
            self.configuration.attachments[side] = None;
            let vertex = if side == 0 { 0 } else { end_index };
            self.inverse_masses[vertex] =
                f32_from_usize(self.points.len()) / self.configuration.mass;
            self.torn = true;
        }
        self.awake = true;
        Ok(())
    }

    pub(crate) fn write_status(&self, output: &mut RopeStatus) {
        output.points.clone_from(&self.points);
        output.velocities.clone_from(&self.velocities);
        output.contact_normals.clone_from(&self.contact_normals);
        output.contact_bodies.clone_from(&self.contact_bodies);
        output.static_contacts = self.static_contacts;
        output.dynamic_contacts = self.dynamic_contacts;
        output.target_pose_mode = self.target_mode;
        output.animation_stiffness = self.configuration.animation_stiffness;
        output.strained = self.strained;
        output
            .subdivided_vertices
            .clone_from(&self.subdivision_vertices);
        output.time_last_active = self.time_last_active;
        output.host_position = self.host_position;
        output.host_rotation = self.host_rotation;
        output.torn = self.torn;
    }

    pub(crate) fn take_reactions(&mut self) -> SmallVec<[DeformableReaction; 16]> {
        core::mem::take(&mut self.reactions)
    }

    pub(crate) fn step<A, C, M>(
        &mut self,
        time_step: f32,
        scene_gravity: Vec3,
        physics_time: f32,
        mut attachment_frame: A,
        mut collide: C,
        mut medium_at: M,
    ) -> Result<(), PhysicsError>
    where
        A: FnMut(RopeAttachment) -> Result<AttachmentFrame, PhysicsError>,
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
        M: FnMut(Vec3) -> MediumSample,
    {
        self.reactions.clear();
        let substeps = convert::substeps(time_step, self.configuration.maximum_time_step);
        let dt = time_step / f32_from_u32(substeps);
        for _ in 0..substeps {
            self.step_inner(
                dt,
                scene_gravity,
                &mut attachment_frame,
                &mut collide,
                &mut medium_at,
            )?;
        }
        let energy = self
            .velocities
            .iter()
            .map(|velocity| velocity.length_squared())
            .sum::<f32>()
            * (self.configuration.mass / f32_from_usize(self.velocities.len()));
        self.awake = energy > self.configuration.minimum_energy
            || self.configuration.attachments.iter().any(Option::is_some);
        if self.awake {
            self.time_last_active = physics_time;
        }
        Ok(())
    }

    fn step_inner<A, C, M>(
        &mut self,
        dt: f32,
        scene_gravity: Vec3,
        attachment_frame: &mut A,
        collide: &mut C,
        medium_at: &mut M,
    ) -> Result<(), PhysicsError>
    where
        A: FnMut(RopeAttachment) -> Result<AttachmentFrame, PhysicsError>,
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
        M: FnMut(Vec3) -> MediumSample,
    {
        let had_contacts = self.static_contacts + self.dynamic_contacts > 0;
        self.clear_contacts();
        self.previous_points.clone_from(&self.points);
        self.integrate_free_motion(dt, scene_gravity, medium_at);

        self.pin_attachments(attachment_frame)?;
        let target_frame = self.target_attachment_frame(attachment_frame)?;
        self.solve_lengths(dt, had_contacts, attachment_frame, target_frame)?;
        self.collide_points(collide, target_frame)?;

        for index in 0..self.points.len() {
            if self.inverse_masses[index] > 0.0 {
                self.velocities[index] = (self.points[index] - self.previous_points[index]) / dt;
            }
        }
        self.apply_target_pose(target_frame, dt);
        self.solve_velocity_constraints(collide)?;
        Ok(())
    }

    /// Drops the previous substep's contact record.
    fn clear_contacts(&mut self) {
        self.static_contacts = 0;
        self.dynamic_contacts = 0;
        self.contact_normals.fill(Vec3::ZERO);
        self.contact_bodies.fill(None);
        self.contact_velocities.fill(Vec3::ZERO);
        self.contact_points.fill(Vec3::ZERO);
        self.contact_dynamic.fill(false);
    }

    /// Applies gravity, the accumulated force and torque, medium drag, and
    /// damping, then advances every unpinned point.
    fn integrate_free_motion<M>(&mut self, dt: f32, scene_gravity: Vec3, medium_at: &mut M)
    where
        M: FnMut(Vec3) -> MediumSample,
    {
        let gravity = self.configuration.gravity.unwrap_or(scene_gravity);
        let force_acceleration = self.pending_force / self.configuration.mass;
        self.pending_force = Vec3::ZERO;
        let torque = core::mem::take(&mut self.pending_torque);
        if torque != Vec3::ZERO {
            self.apply_angular_impulse(torque * dt);
        }
        let damping = self.configuration.damping.mul_add(-dt, 1.0).clamp(0.0, 1.0);
        for index in 0..self.points.len() {
            if self.inverse_masses[index] == 0.0 {
                continue;
            }
            let medium = medium_at(self.points[index]);
            let medium_gravity = medium.gravity.unwrap_or(gravity);
            let local_gravity = gravity.lerp(medium_gravity, medium.submerged_fraction);
            let resistance = lerp_scalar(
                self.configuration.air_resistance,
                self.configuration.water_resistance,
                medium.submerged_fraction,
            );
            let relative_wind = self.configuration.wind + medium.velocity - self.velocities[index];
            self.velocities[index] +=
                (local_gravity + force_acceleration + relative_wind * resistance) * dt;
            self.velocities[index] *= damping;
            self.points[index] += self.velocities[index] * dt;
        }
    }

    /// Runs the native length and joint-limit iteration, then tears the rope
    /// when the accumulated constraint impulse passes the authored maximum
    /// force.
    fn solve_lengths<A>(
        &mut self,
        dt: f32,
        had_contacts: bool,
        attachment_frame: &mut A,
        target_frame: Option<AttachmentFrame>,
    ) -> Result<(), PhysicsError>
    where
        A: FnMut(RopeAttachment) -> Result<AttachmentFrame, PhysicsError>,
    {
        let no_velocity_solver = self
            .configuration
            .flags
            .contains(RopeFlags::NO_VELOCITY_SOLVER);
        let stiffness = if had_contacts
            && self
                .configuration
                .flags
                .contains(RopeFlags::NO_STIFFNESS_WHEN_COLLIDING)
        {
            0.0
        } else {
            self.configuration.stiffness
        };
        let compliance = if stiffness > 0.0 {
            1.0 / (stiffness * dt * dt)
        } else if no_velocity_solver {
            0.0
        } else {
            1.0e-6
        };
        self.strained = false;
        let mut maximum_constraint_impulse = 0.0_f32;
        for _ in 0..self.configuration.maximum_iterations {
            let (maximum_error, maximum_lambda) = self.enforce_lengths(compliance);
            maximum_constraint_impulse =
                maximum_constraint_impulse.max(maximum_lambda / dt.max(f32::EPSILON));
            self.pin_attachments(attachment_frame)?;
            if self.configuration.joint_limit > 0.0
                && self.target_mode != RopeTargetPoseMode::Disabled
                && !self.configuration.attachments.iter().all(Option::is_some)
            {
                self.enforce_joint_limits(target_frame);
            }
            if maximum_error <= 1.0e-4 {
                break;
            }
            self.strained = true;
        }
        if self.strained
            && self.configuration.attachments.iter().all(Option::is_some)
            && self.configuration.maximum_force > 0.0
            && maximum_constraint_impulse > self.configuration.maximum_force * dt.max(0.01)
            && !self.configuration.flags.contains(RopeFlags::NO_TEARS)
        {
            self.configuration.attachments[1] = None;
            let last = self.inverse_masses.len() - 1;
            self.inverse_masses[last] = f32_from_usize(self.points.len()) / self.configuration.mass;
            self.torn = true;
        }
        Ok(())
    }

    /// Sweeps every point, then every segment interior, against the world and
    /// records the resulting contacts.
    fn collide_points<C>(
        &mut self,
        collide: &mut C,
        target_frame: Option<AttachmentFrame>,
    ) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        if !self.configuration.flags.contains(RopeFlags::COLLIDES)
            && !self
                .configuration
                .flags
                .contains(RopeFlags::COLLIDES_WITH_TERRAIN)
        {
            return Ok(());
        }
        for index in 0..self.points.len() {
            let radius = self.segment_thickness(index.saturating_sub(1));
            if let Some(contact) = collide(self.previous_points[index], self.points[index], radius)?
            {
                self.points[index] = contact.position + contact.normal * radius;
                self.contact_normals[index] = contact.normal;
                self.contact_bodies[index] = Some(contact.body);
                self.contact_velocities[index] = contact.velocity;
                self.contact_points[index] = contact.position;
                self.contact_dynamic[index] = contact.dynamic;
                if contact.dynamic {
                    self.dynamic_contacts += 1;
                } else {
                    self.static_contacts += 1;
                }
            }
        }
        self.resolve_segment_collisions(collide, target_frame)
    }

    /// Maps `CRopeEntity::CheckCollisions`' line-unprojection pass onto
    /// Rapier sweeps. Endpoint sweeps above handle temporal motion; this pass
    /// sweeps the thin Cry line radius along each current rope segment so an
    /// obstacle crossing the segment interior cannot be missed.
    fn resolve_segment_collisions<C>(
        &mut self,
        collide: &mut C,
        target_frame: Option<AttachmentFrame>,
    ) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        let reverse = self.configuration.attachments[0].is_none()
            && self.configuration.attachments[1].is_some();
        let segment_count = self.rest_lengths.len();
        let hinge_axis = self
            .configuration
            .hinge_axis
            .map(|axis| target_frame.map_or(axis, |frame| frame.rotation * axis));

        for order in 0..segment_count {
            let segment = if reverse {
                segment_count - 1 - order
            } else {
                order
            };
            let (leading, trailing) = if reverse {
                (segment + 1, segment)
            } else {
                (segment, segment + 1)
            };
            let start = self.points[leading];
            let end = self.points[trailing];
            let full_delta = end - start;
            let full_length = full_delta.length();
            if full_length <= f32::EPSILON {
                continue;
            }
            let skipped_fraction = if order == 0 {
                self.configuration.no_collision_distance
            } else {
                0.0
            };
            let query_start = start + full_delta * skipped_fraction;
            let query_delta = end - query_start;
            let query_length = query_delta.length();
            if query_length <= f32::EPSILON {
                continue;
            }
            let line_radius = self.segment_thickness(segment) * 0.1;
            let Some(contact) = collide(query_start, end, line_radius)? else {
                continue;
            };
            let query_fraction = (contact.distance / query_length).clamp(0.0, 1.0);
            let fraction = skipped_fraction + query_fraction * (1.0 - skipped_fraction);
            let point = start.lerp(end, fraction);
            let target = contact.position + contact.normal * line_radius;
            let correction = target - point;
            if correction.length_squared() <= f32::EPSILON {
                continue;
            }

            if let Some(axis) = hinge_axis {
                let axis = axis.normalize_or_zero();
                let tangent = axis.cross(full_delta);
                let tangent_length = tangent.length();
                if axis != Vec3::ZERO
                    && tangent_length > f32::EPSILON
                    && self.inverse_masses[trailing] > 0.0
                {
                    let maximum_angle = if self.configuration.joint_limit > 0.0 {
                        core::f32::consts::PI
                    } else {
                        self.configuration.unprojection_limit
                    };
                    let angle = (correction.dot(tangent / tangent_length) / full_length)
                        .clamp(-maximum_angle, maximum_angle);
                    self.points[trailing] = start + Quat::from_axis_angle(axis, angle) * full_delta;
                }
            } else {
                let leading_weight = 1.0 - fraction;
                let trailing_weight = fraction;
                let denominator = (self.inverse_masses[trailing] * trailing_weight).mul_add(
                    trailing_weight,
                    self.inverse_masses[leading] * leading_weight * leading_weight,
                );
                if denominator > f32::EPSILON {
                    self.points[leading] +=
                        correction * (self.inverse_masses[leading] * leading_weight / denominator);
                    self.points[trailing] += correction
                        * (self.inverse_masses[trailing] * trailing_weight / denominator);
                }
            }

            let contact_vertex = trailing;
            self.contact_normals[contact_vertex] = contact.normal;
            self.contact_bodies[contact_vertex] = Some(contact.body);
            self.contact_velocities[contact_vertex] = contact.velocity;
            self.contact_points[contact_vertex] = contact.position;
            self.contact_dynamic[contact_vertex] = contact.dynamic;
            if contact.dynamic {
                self.dynamic_contacts += 1;
            } else {
                self.static_contacts += 1;
            }
        }
        Ok(())
    }

    fn pin_attachments<A>(&mut self, attachment_frame: &mut A) -> Result<(), PhysicsError>
    where
        A: FnMut(RopeAttachment) -> Result<AttachmentFrame, PhysicsError>,
    {
        for (side, attachment) in self.configuration.attachments.iter().copied().enumerate() {
            let Some(attachment) = attachment else {
                continue;
            };
            let index = if side == 0 { 0 } else { self.points.len() - 1 };
            let frame = attachment_frame(attachment)?;
            self.points[index] = frame.position;
            self.velocities[index] = frame.velocity;
            self.inverse_masses[index] = 0.0;
        }
        Ok(())
    }

    fn target_attachment_frame<A>(
        &mut self,
        attachment_frame: &mut A,
    ) -> Result<Option<AttachmentFrame>, PhysicsError>
    where
        A: FnMut(RopeAttachment) -> Result<AttachmentFrame, PhysicsError>,
    {
        let side = if self
            .configuration
            .flags
            .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_END)
        {
            Some(1)
        } else if self
            .configuration
            .flags
            .contains(RopeFlags::TARGET_VERTEX_RELATIVE_TO_START)
        {
            Some(0)
        } else {
            None
        };
        let Some(side) = side else {
            return Ok(None);
        };
        let Some(attachment) = self.configuration.attachments[side] else {
            self.host_position = Vec3::ZERO;
            self.host_rotation = Quat::IDENTITY;
            return Ok(None);
        };
        let frame = attachment_frame(attachment)?;
        self.host_position = frame.position;
        self.host_rotation = frame.rotation;
        Ok(Some(frame))
    }

    fn apply_target_pose(&mut self, frame: Option<AttachmentFrame>, dt: f32) {
        if !self.target_initialized {
            return;
        }
        match self.target_mode {
            RopeTargetPoseMode::Disabled => {}
            RopeTargetPoseMode::DirectVertexPull => self.pull_toward_target(frame, dt),
            RopeTargetPoseMode::JointTorque => self.twist_toward_target(frame, dt),
        }
    }

    /// `RopeTargetPoseMode::DirectVertexPull`: every unpinned point is pulled
    /// toward its target position at its own segment's stiffness.
    fn pull_toward_target(&mut self, frame: Option<AttachmentFrame>, dt: f32) {
        let target = |index: usize| {
            frame.map_or(self.target_points[index], |frame| {
                frame.position + frame.rotation * self.target_points[index]
            })
        };
        let segment_count = self.points.len() - 1;
        for index in 1..self.points.len() {
            if self.inverse_masses[index] == 0.0 {
                continue;
            }
            let segment = index - 1;
            let stiffness = self.segment_stiffness(segment);
            let damping = self.segment_damping(segment);
            let decay = 1.0
                - self.configuration.animation_stiffness_decay * f32_from_usize(index)
                    / f32_from_usize(segment_count);
            let requested_velocity = (target(index) - self.points[index]) * (stiffness * decay);
            let retained = (1.0 - damping * dt).max(0.0);
            self.velocities[index] =
                self.velocities[index] * retained + requested_velocity * (1.0 - retained);
        }
    }

    /// `RopeTargetPoseMode::JointTorque`: each joint is driven by the bend and
    /// twist between its current frame and its target frame, walking the rope
    /// from the attached end.
    fn twist_toward_target(&mut self, frame: Option<AttachmentFrame>, dt: f32) {
        let target = |index: usize| {
            frame.map_or(self.target_points[index], |frame| {
                frame.position + frame.rotation * self.target_points[index]
            })
        };
        if self.configuration.animation_stiffness <= 0.0 {
            for index in 0..self.points.len() {
                if self.inverse_masses[index] > 0.0 {
                    self.points[index] = target(index);
                }
            }
            return;
        }

        let segment_count = self.points.len() - 1;
        let mut previous_current_direction = Vec3::Z;
        let mut previous_target_direction = Vec3::Z;
        let mut previous_current_axis = Vec3::Y;
        let mut previous_target_axis = Vec3::Y;
        let axis_fallback_threshold = self.rest_lengths.iter().sum::<f32>() * 0.001;
        if self.configuration.attachments[0].is_some() {
            let host_position = frame.map_or(self.host_position, |frame| frame.position);
            previous_target_direction = (self.points[0] - host_position).normalize_or_zero();
            let first_target_direction = target(1) - target(0);
            if previous_target_direction
                .dot(first_target_direction.normalize_or_zero())
                .abs()
                > 0.985
            {
                previous_target_direction = orthogonal(first_target_direction);
            }
            previous_current_direction = previous_target_direction;
            previous_current_axis = orthogonal(previous_current_direction);
            previous_target_axis = previous_current_axis;
        }

        for segment in 0..segment_count {
            let current_direction =
                (self.points[segment + 1] - self.points[segment]).normalize_or_zero();
            let target_direction = (target(segment + 1) - target(segment)).normalize_or_zero();
            let current_cross = previous_current_direction.cross(current_direction);
            let target_cross = previous_target_direction.cross(target_direction);
            let current_cross_length = current_cross.length();
            let target_cross_length = target_cross.length();
            let current_angle =
                current_cross_length.atan2(previous_current_direction.dot(current_direction));
            let target_angle =
                target_cross_length.atan2(previous_target_direction.dot(target_direction));
            let current_axis = if current_cross_length > axis_fallback_threshold {
                current_cross / current_cross_length
            } else {
                previous_current_axis
            };
            let target_axis = if target_cross_length > axis_fallback_threshold {
                target_cross / target_cross_length
            } else {
                previous_target_axis
            };
            let bend_delta = wrap_angle(target_angle - current_angle);
            let current_twist = previous_current_direction
                .dot(current_axis.cross(previous_current_axis))
                .atan2(
                    previous_current_axis
                        .dot(previous_current_direction)
                        .mul_add(
                            -current_axis.dot(previous_current_direction),
                            previous_current_axis.dot(current_axis),
                        ),
                );
            let target_twist = previous_target_direction
                .dot(target_axis.cross(previous_target_axis))
                .atan2(previous_target_axis.dot(previous_target_direction).mul_add(
                    -target_axis.dot(previous_target_direction),
                    previous_target_axis.dot(target_axis),
                ));
            let angular_error = current_axis * bend_delta
                - previous_current_direction * wrap_angle(target_twist - current_twist);
            let stiffness = self.segment_stiffness(segment);
            let damping = self.segment_damping(segment);
            let decay = 1.0
                - self.configuration.animation_stiffness_decay * f32_from_usize(segment + 1)
                    / f32_from_usize(segment_count);
            let segment_vector = self.points[segment + 1] - self.points[segment];
            let velocity_delta = (angular_error * (stiffness * decay * dt)).cross(segment_vector);
            let relative_velocity = self.velocities[segment + 1] - self.velocities[segment];
            self.velocities[segment + 1] = self.velocities[segment]
                + relative_velocity * damping.mul_add(-dt, 1.0).max(0.0)
                + velocity_delta;

            previous_current_direction = current_direction;
            previous_target_direction = target_direction;
            previous_current_axis = current_axis;
            previous_target_axis = target_axis;
        }
    }

    fn enforce_lengths(&mut self, compliance: f32) -> (f32, f32) {
        let mut maximum_error = 0.0_f32;
        let mut maximum_lambda = 0.0_f32;
        for index in 0..self.rest_lengths.len() {
            let delta = self.points[index + 1] - self.points[index];
            let length = delta.length();
            if length <= f32::EPSILON {
                continue;
            }
            let error = length - self.rest_lengths[index];
            maximum_error = maximum_error.max(error.abs());
            let inverse_mass_a = self.inverse_masses[index];
            let inverse_mass_b = self.inverse_masses[index + 1];
            let denominator = inverse_mass_a + inverse_mass_b + compliance;
            if denominator <= f32::EPSILON {
                continue;
            }
            let correction = delta / length * (error / denominator);
            maximum_lambda = maximum_lambda.max((error / denominator).abs());
            self.points[index] += correction * inverse_mass_a;
            self.points[index + 1] -= correction * inverse_mass_b;
        }
        (maximum_error, maximum_lambda)
    }

    fn enforce_joint_limits(&mut self, frame: Option<AttachmentFrame>) {
        let target = |index: usize| {
            frame.map_or(self.target_points[index], |frame| {
                frame.position + frame.rotation * self.target_points[index]
            })
        };
        let segment_count = self.points.len() - 1;
        let plane_normal = self
            .configuration
            .hinge_axis
            .map(|axis| frame.map_or(axis, |frame| frame.rotation * axis));
        for index in 1..self.points.len() - 1 {
            let previous = self.points[index] - self.points[index - 1];
            let next = self.points[index + 1] - self.points[index];
            let target_previous = target(index) - target(index - 1);
            let target_next = target(index + 1) - target(index);
            let axis = previous.cross(next);
            let axis_length = axis.length();
            let current_angle = axis_length.atan2(previous.dot(next));
            let target_angle = target_previous
                .cross(target_next)
                .length()
                .atan2(target_previous.dot(target_next));
            let decay = self.configuration.joint_limit_decay / f32_from_usize(segment_count.max(1));
            let limit =
                self.configuration.joint_limit * (1.0 - decay * f32_from_usize(index)).max(0.0);
            let difference = wrap_angle(target_angle - current_angle);
            let mut direction = next.normalize_or_zero();
            if difference.abs() > limit && axis_length > 1.0e-20 {
                let correction = difference - limit * difference.signum();
                direction = Quat::from_axis_angle(axis / axis_length, correction) * direction;
            }
            if let Some(normal) = plane_normal {
                direction = (direction - normal * normal.dot(direction)).normalize_or_zero();
            }
            if direction != Vec3::ZERO && self.inverse_masses[index + 1] > 0.0 {
                self.points[index + 1] = self.points[index] + direction * self.rest_lengths[index];
            }
        }
    }

    fn segment_thickness(&self, segment: usize) -> f32 {
        self.configuration
            .segments
            .get(segment)
            .and_then(|segment| segment.thickness)
            .unwrap_or(self.configuration.collision_distance)
            .max(1.0e-4)
    }

    fn segment_stiffness(&self, segment: usize) -> f32 {
        self.configuration
            .segments
            .get(segment)
            .and_then(|segment| segment.stiffness)
            .unwrap_or(self.configuration.animation_stiffness)
    }

    fn segment_damping(&self, segment: usize) -> f32 {
        self.configuration
            .segments
            .get(segment)
            .and_then(|segment| segment.damping)
            .unwrap_or(self.configuration.animation_damping)
    }

    fn solve_velocity_constraints<C>(&mut self, collide: &mut C) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        let subdivided = self
            .configuration
            .flags
            .contains(RopeFlags::SUBDIVIDE_SEGMENTS);
        let mut solver_vertices = if subdivided {
            self.build_dynamic_subdivision(collide)?
        } else {
            Vec::new()
        };

        let solver_enabled = !self
            .configuration
            .flags
            .contains(RopeFlags::NO_VELOCITY_SOLVER)
            && self.configuration.target_length > 0.0;
        if solver_enabled {
            let energy_before = self
                .velocities
                .iter()
                .map(|velocity| velocity.length_squared())
                .sum::<f32>();
            if subdivided {
                self.solve_subdivided_multicontact(&mut solver_vertices);
            } else if self.static_contacts + self.dynamic_contacts > 0 {
                self.solve_main_multicontact();
            } else {
                self.solve_direct_axial_velocities();
            }
            self.clamp_solver_energy(energy_before);
        }

        self.subdivision_vertices.clear();
        if subdivided {
            self.subdivision_vertices
                .extend(solver_vertices.iter().map(|vertex| vertex.position));
        }
        Ok(())
    }

    fn build_dynamic_subdivision<C>(
        &mut self,
        collide: &mut C,
    ) -> Result<Vec<RopeSolverVertex>, PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        let maximum_insertions = self.configuration.maximum_subdivision_vertices as usize;
        let mut vertices = Vec::with_capacity(
            self.points.len() + maximum_insertions.saturating_mul(self.rest_lengths.len()),
        );

        for segment in 0..self.rest_lengths.len() {
            let mut segment_vertices = Vec::with_capacity(maximum_insertions + 2);
            segment_vertices.push(RopeSolverVertex::main(
                self.points[segment],
                self.velocities[segment],
                segment,
                segment,
                self.main_solver_contact(segment),
            ));
            segment_vertices.push(RopeSolverVertex::main(
                self.points[segment + 1],
                self.velocities[segment + 1],
                segment,
                segment + 1,
                self.main_solver_contact(segment + 1),
            ));

            let minimum_separation = self.rest_lengths[segment].max(1.0e-4) * 0.05;
            let minimum_separation_squared = minimum_separation * minimum_separation;
            let radius = self.segment_thickness(segment);
            let source_delta = self.points[segment + 1] - self.points[segment];
            let source_length_squared = source_delta.length_squared();
            let mut inserted = 0;
            let mut incomplete_passes = 0;

            while inserted < maximum_insertions && incomplete_passes < 3 {
                let mut edges = (0..segment_vertices.len() - 1).collect::<Vec<_>>();
                edges.sort_by(|left, right| {
                    let left_length = (segment_vertices[*left + 1].position
                        - segment_vertices[*left].position)
                        .length_squared();
                    let right_length = (segment_vertices[*right + 1].position
                        - segment_vertices[*right].position)
                        .length_squared();
                    right_length.total_cmp(&left_length)
                });

                let mut contact_inserted = false;
                for edge in edges {
                    let start = segment_vertices[edge].position;
                    let end = segment_vertices[edge + 1].position;
                    let Some(contact) = collide(start, end, radius)? else {
                        continue;
                    };
                    let position = contact.position + contact.normal * radius;
                    if position.distance_squared(start) < minimum_separation_squared
                        || position.distance_squared(end) < minimum_separation_squared
                    {
                        continue;
                    }
                    let fraction = if source_length_squared > f32::EPSILON {
                        ((contact.position - self.points[segment]).dot(source_delta)
                            / source_length_squared)
                            .clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    let velocity =
                        self.velocities[segment].lerp(self.velocities[segment + 1], fraction);
                    segment_vertices.insert(
                        edge + 1,
                        RopeSolverVertex::contact(position, velocity, segment, contact),
                    );
                    if contact.dynamic {
                        self.dynamic_contacts += 1;
                    } else {
                        self.static_contacts += 1;
                    }
                    inserted += 1;
                    incomplete_passes = 0;
                    contact_inserted = true;
                    break;
                }
                if !contact_inserted {
                    incomplete_passes += 1;
                }
            }

            if vertices.is_empty() {
                vertices.push(segment_vertices[0]);
            } else if let Some(boundary) = vertices.last_mut() {
                boundary.source_segment = segment;
            }
            vertices.extend(segment_vertices.into_iter().skip(1));
        }
        Ok(vertices)
    }

    fn main_solver_contact(&self, index: usize) -> Option<DeformableContact> {
        let body = self.contact_bodies[index]?;
        Some(DeformableContact {
            position: self.contact_points[index],
            normal: self.contact_normals[index],
            distance: 0.0,
            velocity: self.contact_velocities[index],
            body,
            dynamic: self.contact_dynamic[index],
        })
    }

    fn solve_subdivided_multicontact(&mut self, vertices: &mut [RopeSolverVertex]) {
        if vertices.len() < 2 {
            return;
        }
        let segment_count = self.rest_lengths.len();
        let mut path_lengths = vec![0.0_f32; segment_count];
        let mut total_length = 0.0;
        for edge in 0..vertices.len() - 1 {
            let length = vertices[edge]
                .position
                .distance(vertices[edge + 1].position);
            total_length += length;
            path_lengths[vertices[edge].source_segment] += length;
        }
        if total_length <= f32::EPSILON {
            return;
        }
        let stiffness = self.configuration.stiffness / total_length;
        let requested_velocities = path_lengths
            .into_iter()
            .map(|path_length| {
                stiffness
                    * (self.configuration.target_length
                        - (path_length * f32_from_usize(segment_count)).min(total_length * 1.5))
            })
            .collect::<Vec<_>>();
        let start_attached = self.configuration.attachments[0].is_some();
        let end_attached = self.configuration.attachments[1].is_some();
        let last_edge = vertices.len() - 2;
        let mut remaining = self.configuration.maximum_iterations as usize;

        while remaining > 0 {
            let mut corrections = 0;
            for index in 0..vertices.len() {
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
                if index < vertices.len() - 1 {
                    let delta = vertices[index + 1].position - vertices[index].position;
                    let direction = delta.normalize_or_zero();
                    let requested =
                        delta.dot(direction) * requested_velocities[vertices[index].source_segment];
                    let relative = (vertices[index + 1].velocity - vertices[index].velocity)
                        .dot(direction)
                        - requested;
                    if relative.abs() > CRY_MULTICONTACT_ACCURACY {
                        let left_weight = if index == 0 {
                            if start_attached { 0.0 } else { 0.5 }
                        } else if index == last_edge {
                            if end_attached { 1.0 } else { 0.5 }
                        } else {
                            0.5
                        };
                        vertices[index].velocity += direction * (relative * left_weight);
                        vertices[index + 1].velocity -=
                            direction * (relative * (1.0 - left_weight));
                        corrections += 1;
                    }
                }
                if Self::resolve_rope_contact(&mut vertices[index], self.configuration.friction) {
                    corrections += 4;
                }
            }
            if corrections == 0 || remaining <= corrections {
                break;
            }
            remaining -= corrections;
        }

        for vertex in vertices.iter() {
            if let Some(index) = vertex.main_vertex {
                self.velocities[index] = vertex.velocity;
            }
            let Some(contact) = vertex.contact.filter(|contact| contact.dynamic) else {
                continue;
            };
            if vertex.contact_velocity_delta == Vec3::ZERO {
                continue;
            }
            let mass = vertex.main_vertex.map_or_else(
                || self.configuration.mass / f32_from_usize(self.points.len()),
                |index| {
                    if self.inverse_masses[index] > 0.0 {
                        self.inverse_masses[index].recip()
                    } else {
                        0.0
                    }
                },
            );
            let impulse = -vertex.contact_velocity_delta * mass;
            if impulse.is_finite() && impulse != Vec3::ZERO {
                self.reactions.push(DeformableReaction {
                    body: contact.body,
                    point: contact.position,
                    impulse,
                });
            }
        }
    }

    fn solve_main_multicontact(&mut self) {
        let last_segment = self.rest_lengths.len() - 1;
        let mut vertices = (0..self.points.len())
            .map(|index| {
                RopeSolverVertex::main(
                    self.points[index],
                    self.velocities[index],
                    index.min(last_segment),
                    index,
                    self.main_solver_contact(index),
                )
            })
            .collect::<Vec<_>>();
        let segment_length =
            self.configuration.target_length / f32_from_usize(self.rest_lengths.len());
        let axial_tolerance = segment_length * 0.005;
        let mut remaining = self.configuration.maximum_iterations as usize;
        while remaining > 0 {
            let mut corrections = 0;
            for index in 0..vertices.len() {
                if remaining == 0 {
                    break;
                }
                remaining -= 1;
                if index < vertices.len() - 1 {
                    let direction = (vertices[index + 1].position - vertices[index].position)
                        .normalize_or_zero();
                    let relative =
                        (vertices[index + 1].velocity - vertices[index].velocity).dot(direction);
                    if relative.abs() > axial_tolerance {
                        let left_weight = if self.inverse_masses[index] > 0.0 {
                            0.5
                        } else {
                            0.0
                        };
                        let right_weight = if self.inverse_masses[index + 1] > 0.0 {
                            0.5
                        } else {
                            0.0
                        };
                        vertices[index].velocity += direction * (relative * left_weight);
                        vertices[index + 1].velocity -= direction * (relative * right_weight);
                        corrections += 1;
                    }
                }
                if Self::resolve_rope_contact(&mut vertices[index], self.configuration.friction) {
                    corrections += 4;
                }
            }
            if corrections == 0 || remaining <= corrections {
                break;
            }
            remaining -= corrections;
        }
        for vertex in vertices {
            let index = vertex.main_vertex.expect("main rope solver vertex");
            self.velocities[index] = vertex.velocity;
            let Some(contact) = vertex.contact.filter(|contact| contact.dynamic) else {
                continue;
            };
            if self.inverse_masses[index] <= 0.0 {
                continue;
            }
            let impulse = -vertex.contact_velocity_delta * self.inverse_masses[index].recip();
            if impulse.is_finite() && impulse != Vec3::ZERO {
                self.reactions.push(DeformableReaction {
                    body: contact.body,
                    point: contact.position,
                    impulse,
                });
            }
        }
    }

    fn resolve_rope_contact(vertex: &mut RopeSolverVertex, friction: f32) -> bool {
        let Some(contact) = vertex.contact else {
            return false;
        };
        let mut correction = vertex.velocity - contact.velocity;
        let normal_velocity = correction.dot(contact.normal);
        if normal_velocity >= -CRY_MULTICONTACT_ACCURACY {
            return false;
        }
        if friction > 0.01 {
            let tangent = correction - contact.normal * normal_velocity;
            let tangent_speed = tangent.length();
            vertex.friction_impulse -= tangent_speed;
            let remaining_friction = normal_velocity.mul_add(-friction, vertex.friction_impulse);
            if remaining_friction < 0.0 {
                if tangent_speed > f32::EPSILON {
                    correction += tangent * (remaining_friction / tangent_speed);
                }
                vertex.friction_impulse = 0.0;
            } else {
                vertex.friction_impulse = remaining_friction;
            }
        } else {
            correction = contact.normal * normal_velocity;
        }
        let before = vertex.velocity;
        vertex.velocity -= correction;
        vertex.contact_velocity_delta += vertex.velocity - before;
        true
    }

    fn solve_direct_axial_velocities(&mut self) {
        let segment_count = self.rest_lengths.len();
        let directions = self
            .points
            .windows(2)
            .map(|points| (points[1] - points[0]).normalize_or_zero())
            .collect::<Vec<_>>();
        if segment_count == 1 {
            let relative = (self.velocities[1] - self.velocities[0]).dot(directions[0]);
            let left_dynamic = self.inverse_masses[0] > 0.0;
            let right_dynamic = self.inverse_masses[1] > 0.0;
            let denominator = usize::from(left_dynamic) + usize::from(right_dynamic);
            if denominator > 0 {
                let impulse = relative / f32_from_usize(denominator);
                if left_dynamic {
                    self.velocities[0] += directions[0] * impulse;
                }
                if right_dynamic {
                    self.velocities[1] -= directions[0] * impulse;
                }
            }
            return;
        }

        let mut lower = vec![0.0_f32; segment_count];
        let mut upper = vec![0.0_f32; segment_count];
        let mut right_hand_side = vec![0.0_f32; segment_count];
        upper[0] = directions[1].dot(directions[0]);
        right_hand_side[0] = (self.velocities[0] - self.velocities[1]).dot(directions[0]);
        if self.configuration.attachments[0].is_some() {
            upper[0] *= 2.0;
            right_hand_side[0] *= 2.0;
        }
        for index in 1..segment_count {
            lower[index] = directions[index - 1].dot(directions[index]);
            if index + 1 < segment_count {
                upper[index] = directions[index + 1].dot(directions[index]);
            }
            right_hand_side[index] =
                (self.velocities[index] - self.velocities[index + 1]).dot(directions[index]);
        }
        if self.configuration.attachments[1].is_some() {
            let last = segment_count - 1;
            lower[last] *= 2.0;
            right_hand_side[last] *= 2.0;
        }

        upper[0] *= -0.5;
        right_hand_side[0] *= -0.5;
        for index in 1..segment_count {
            let diagonal = lower[index].mul_add(-upper[index - 1], -2.0);
            let inverse = if diagonal.abs() > 1.0e-10 {
                diagonal.recip()
            } else {
                0.0
            };
            upper[index] *= inverse;
            right_hand_side[index] =
                right_hand_side[index - 1].mul_add(-lower[index], right_hand_side[index]) * inverse;
        }
        let mut impulses = vec![0.0_f32; segment_count];
        impulses[segment_count - 1] = right_hand_side[segment_count - 1];
        for index in (0..segment_count - 1).rev() {
            impulses[index] = upper[index].mul_add(-impulses[index + 1], right_hand_side[index]);
        }

        for index in 0..segment_count {
            if self.inverse_masses[index] > 0.0 {
                self.velocities[index] += directions[index] * impulses[index];
            }
            if self.inverse_masses[index + 1] > 0.0 {
                self.velocities[index + 1] -= directions[index] * impulses[index];
            }
        }
    }

    fn clamp_solver_energy(&mut self, energy_before: f32) {
        let energy_after = self
            .velocities
            .iter()
            .map(|velocity| velocity.length_squared())
            .sum::<f32>();
        if energy_after > energy_before && energy_after > self.configuration.minimum_energy {
            let scale = (energy_before / energy_after).max(0.0).sqrt();
            for velocity in &mut self.velocities {
                *velocity *= scale;
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct SoftEdge {
    vertices: [usize; 2],
    rest_length: f32,
}

#[derive(Debug, Clone)]
struct SoftFan {
    vertex: usize,
    neighbors: Vec<usize>,
    full: bool,
    rest_pair_angles: Vec<f32>,
    rest_normal_angle: f32,
}

#[derive(Debug, Clone)]
struct SoftRigidCore {
    indices: Vec<usize>,
    local_offsets: Vec<Vec3>,
    center: Vec3,
    rotation: Quat,
    mass: f32,
    native_body: Option<RigidBodyHandle>,
    native_collider: Option<ColliderHandle>,
    has_contacts: bool,
    reactions: SmallVec<[SoftRigidCoreReaction; 16]>,
}

#[derive(Debug, Clone, Copy)]
pub struct SoftRigidCoreReaction {
    pub point: Vec3,
    pub impulse: Vec3,
}

#[derive(Debug, Clone)]
pub struct SoftRigidCoreDescriptor {
    pub pose: PhysicsPose,
    pub collider: az_physics::ColliderConfiguration,
}

#[derive(Debug, Clone)]
pub struct SoftRigidCoreUpdate {
    pub body: RigidBodyHandle,
    pub collider: ColliderHandle,
    pub pose: PhysicsPose,
    pub mass: f32,
    pub fit_to_soft_body: bool,
    pub reactions: SmallVec<[SoftRigidCoreReaction; 16]>,
}

#[derive(Debug, Clone)]
pub struct SoftBodyState {
    pub configuration: SoftBodyConfiguration,
    pub query_collider: ColliderHandle,
    pub vertices: Vec<Vec3>,
    pub previous_vertices: Vec<Vec3>,
    pub velocities: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub areas: Vec<f32>,
    pub awake: bool,
    pub host_position: Vec3,
    pub host_rotation: Quat,
    pub entity_pose: PhysicsPose,
    host_initialized: bool,
    host_center: Vec3,
    host_linear_velocity: Vec3,
    host_angular_velocity: Vec3,
    target_vertices: Vec<Vec3>,
    inverse_masses: Vec<f32>,
    masses: Vec<f32>,
    edges: Vec<SoftEdge>,
    fans: Vec<SoftFan>,
    levels: Vec<u32>,
    connected_to_attachment: Vec<bool>,
    maximum_level_reciprocal: f32,
    attachments: BTreeMap<u32, SoftBodyAttachment>,
    pending_force: Vec3,
    pending_torque: Vec3,
    wind_previous: Vec3,
    wind_next: Vec3,
    wind_timer: f32,
    wind_seed: u32,
    stretch_stiffness: f32,
    rigid_core: Option<SoftRigidCore>,
    reactions: SmallVec<[DeformableReaction; 16]>,
}

impl SoftBodyState {
    pub(crate) fn new(
        configuration: SoftBodyConfiguration,
        pose: PhysicsPose,
        query_collider: ColliderHandle,
    ) -> Self {
        let vertices: Vec<_> = configuration
            .vertices
            .iter()
            .map(|vertex| pose.transform_point(*vertex))
            .collect();
        let velocities = if configuration.velocities.is_empty() {
            vec![Vec3::ZERO; vertices.len()]
        } else {
            configuration
                .velocities
                .iter()
                .map(|velocity| pose.rotation * *velocity)
                .collect()
        };
        let target_vertices = if configuration.target_vertices.is_empty() {
            configuration.vertices.clone()
        } else {
            configuration.target_vertices.clone()
        };
        let (edges, fans) = if configuration.tetrahedra.is_empty() {
            soft_topology(&vertices, &configuration.triangles, configuration.flags)
        } else {
            (
                soft_lattice_edges(&vertices, &configuration.tetrahedra),
                Vec::new(),
            )
        };
        let attachments: BTreeMap<_, _> = configuration
            .attachments
            .iter()
            .map(|attachment| (attachment.vertex, *attachment))
            .collect();
        let (levels, connected_to_attachment) =
            soft_levels(vertices.len(), &edges, attachments.keys().copied());
        let vertex_count = vertices.len();
        let maximum_level = levels.iter().copied().max().unwrap_or(0);
        let wind = configuration.wind;
        let mut state = Self {
            configuration,
            query_collider,
            previous_vertices: vertices.clone(),
            velocities,
            normals: vec![Vec3::ZERO; vertex_count],
            areas: vec![0.0; vertex_count],
            vertices,
            awake: true,
            host_position: pose.translation,
            host_rotation: pose.rotation,
            entity_pose: pose,
            host_initialized: false,
            host_center: pose.translation,
            host_linear_velocity: Vec3::ZERO,
            host_angular_velocity: Vec3::ZERO,
            target_vertices,
            inverse_masses: vec![0.0; vertex_count],
            masses: vec![0.0; vertex_count],
            edges,
            fans,
            levels,
            connected_to_attachment,
            maximum_level_reciprocal: 1.0 / f32_from_u32(maximum_level.max(1)),
            attachments,
            pending_force: Vec3::ZERO,
            pending_torque: Vec3::ZERO,
            wind_previous: wind,
            wind_next: wind,
            wind_timer: 0.0,
            wind_seed: u32_from_usize(vertex_count) ^ 0xa341_316c,
            stretch_stiffness: 0.0,
            rigid_core: None,
            reactions: SmallVec::new(),
        };
        state.recalculate_mass_distribution();
        state.recalculate_stretch_stiffness();
        state.initialize_rigid_core();
        state
    }

    pub(crate) fn set_pose(&mut self, pose: PhysicsPose) {
        let delta = pose * self.entity_pose.inverse();
        for vertex in &mut self.vertices {
            *vertex = delta.transform_point(*vertex);
        }
        for vertex in &mut self.previous_vertices {
            *vertex = delta.transform_point(*vertex);
        }
        for velocity in &mut self.velocities {
            *velocity = delta.rotation * *velocity;
        }
        self.entity_pose = pose;
        self.host_position = delta.transform_point(self.host_position);
        self.host_rotation = (delta.rotation * self.host_rotation).normalize();
        self.host_center = delta.transform_point(self.host_center);
        self.host_linear_velocity = delta.rotation * self.host_linear_velocity;
        self.host_angular_velocity = delta.rotation * self.host_angular_velocity;
        if let Some(core) = &mut self.rigid_core {
            core.center = delta.transform_point(core.center);
            core.rotation = (delta.rotation * core.rotation).normalize();
        }
        self.awake = true;
    }

    pub(crate) fn reset(&mut self) {
        for (vertex, source) in self.vertices.iter_mut().zip(&self.configuration.vertices) {
            *vertex = self.entity_pose.transform_point(*source);
        }
        self.previous_vertices.clone_from(&self.vertices);
        if self.configuration.velocities.is_empty() {
            self.velocities.fill(Vec3::ZERO);
        } else {
            for (velocity, source) in self
                .velocities
                .iter_mut()
                .zip(&self.configuration.velocities)
            {
                *velocity = self.entity_pose.rotation * *source;
            }
        }
        self.pending_force = Vec3::ZERO;
        self.pending_torque = Vec3::ZERO;
        self.wind_previous = self.configuration.wind;
        self.wind_next = self.configuration.wind;
        self.wind_timer = 0.0;
        self.host_center = self.host_position;
        self.host_linear_velocity = Vec3::ZERO;
        self.host_angular_velocity = Vec3::ZERO;
        self.reactions.clear();
        self.recalculate_normals();
        self.initialize_rigid_core();
        self.awake = true;
    }

    pub(crate) fn set_velocity(&mut self, velocity: Vec3) {
        for (value, inverse_mass) in self.velocities.iter_mut().zip(&self.inverse_masses) {
            if *inverse_mass > 0.0 {
                *value = velocity;
            }
        }
        self.awake = true;
    }

    pub(crate) fn apply_angular_impulse(&mut self, impulse: Vec3) {
        let (_, radius) = deformable_bounds(&self.vertices);
        let inertia = (self.configuration.mass * radius * radius).max(1.0e-6);
        self.add_angular_velocity(impulse / inertia);
    }

    pub(crate) fn set_angular_velocity(&mut self, angular_velocity: Vec3) {
        let (center, _) = deformable_bounds(&self.vertices);
        for ((velocity, vertex), inverse_mass) in self
            .velocities
            .iter_mut()
            .zip(&self.vertices)
            .zip(&self.inverse_masses)
        {
            if *inverse_mass > 0.0 {
                *velocity = angular_velocity.cross(*vertex - center);
            }
        }
        self.awake = true;
    }

    fn add_angular_velocity(&mut self, angular_velocity: Vec3) {
        let (center, _) = deformable_bounds(&self.vertices);
        for ((velocity, vertex), inverse_mass) in self
            .velocities
            .iter_mut()
            .zip(&self.vertices)
            .zip(&self.inverse_masses)
        {
            if *inverse_mass > 0.0 {
                *velocity += angular_velocity.cross(*vertex - center);
            }
        }
        self.awake = true;
    }

    pub(crate) fn add_force(&mut self, force: Vec3) {
        self.pending_force += force;
        self.awake = true;
    }

    pub(crate) fn add_torque(&mut self, torque: Vec3) {
        self.pending_torque += torque;
        self.awake = true;
    }

    pub(crate) fn set_mass(&mut self, mass: f32) {
        self.configuration.mass = mass;
        self.recalculate_mass_distribution();
        self.recalculate_stretch_stiffness();
        self.initialize_rigid_core();
    }

    pub(crate) fn update_attachments(
        &mut self,
        update: &SoftBodyAttachmentUpdate,
    ) -> Result<(), PhysicsError> {
        update.validate()?;
        for (index, vertex) in update.vertices.iter().copied().enumerate() {
            let vertex_index = vertex as usize;
            if vertex_index >= self.vertices.len() {
                return Err(PhysicsError::SoftBodyVertexNotFound { vertex });
            }
            if update.attached {
                let point = update
                    .points
                    .get(index)
                    .copied()
                    .unwrap_or(self.vertices[vertex_index]);
                self.attachments.insert(
                    vertex,
                    SoftBodyAttachment {
                        vertex,
                        body: update.body,
                        part_id: update.part_id,
                        point,
                        local: update.local,
                    },
                );
            } else {
                self.attachments.remove(&vertex);
            }
        }
        self.host_initialized = false;
        self.recalculate_attachment_levels();
        self.recalculate_mass_distribution();
        self.awake = true;
        Ok(())
    }

    pub(crate) fn first_attachment(&self) -> Option<SoftBodyAttachment> {
        self.attachments.values().next().copied()
    }

    pub(crate) fn set_target(
        &mut self,
        action: &DeformableTargetVertices,
        resolved_host: Option<AttachmentFrame>,
    ) -> Result<(), PhysicsError> {
        action.validate()?;
        if action
            .points
            .as_ref()
            .is_some_and(|points| points.len() != self.vertices.len())
        {
            return Err(PhysicsError::InvalidSoftBodyConfiguration {
                field: "target vertex count",
            });
        }

        let host = action
            .host
            .map(|pose| AttachmentFrame {
                position: pose.translation,
                body_position: pose.translation,
                center: pose.translation,
                rotation: pose.rotation,
                velocity: Vec3::ZERO,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            })
            .or(resolved_host)
            .unwrap_or(AttachmentFrame {
                position: Vec3::ZERO,
                body_position: Vec3::ZERO,
                center: Vec3::ZERO,
                rotation: Quat::IDENTITY,
                velocity: Vec3::ZERO,
                linear_velocity: Vec3::ZERO,
                angular_velocity: Vec3::ZERO,
            });
        let inverse_host_rotation = host.rotation.inverse();
        if let Some(points) = &action.points {
            for (target, point) in self.target_vertices.iter_mut().zip(points) {
                *target = inverse_host_rotation * (*point - host.position);
            }
            if self.configuration.maximum_safe_step > 0.0 {
                for edge in &mut self.edges {
                    let original_length_squared = edge.rest_length * edge.rest_length;
                    let new_length_squared = self.target_vertices[edge.vertices[0]]
                        .distance_squared(self.target_vertices[edge.vertices[1]]);
                    let error_squared =
                        (edge.rest_length * self.configuration.maximum_safe_step * 0.5).powi(2);
                    let discriminant =
                        (original_length_squared + new_length_squared - error_squared).mul_add(
                            original_length_squared + new_length_squared - error_squared,
                            -(original_length_squared * new_length_squared * 4.0),
                        );
                    if discriminant
                        .min(original_length_squared + new_length_squared - error_squared)
                        > 0.0
                    {
                        edge.rest_length = new_length_squared.sqrt();
                    }
                }
            }
        } else {
            for index in 0..self.vertices.len() {
                if !self.attachments.contains_key(&u32_from_usize(index)) {
                    self.target_vertices[index] =
                        inverse_host_rotation * (self.vertices[index] - host.position);
                }
            }
        }
        self.host_position = host.position;
        self.host_rotation = host.rotation;
        self.host_initialized = true;
        self.awake = true;
        Ok(())
    }

    pub(crate) fn apply_impulse(&mut self, impulse: SoftBodyImpulse) -> Result<(), PhysicsError> {
        impulse.validate()?;
        let point = impulse
            .point
            .unwrap_or_else(|| deformable_bounds(&self.vertices).0);
        let (triangle, weights) = if let Some(triangle) = impulse.triangle {
            let triangle_index = triangle as usize;
            let Some(indices) = self.configuration.triangles.get(triangle_index).copied() else {
                return Err(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "impulse triangle",
                });
            };
            let vertices = indices.map(|index| self.vertices[index as usize]);
            (indices, triangle_barycentric_weights(point, vertices))
        } else {
            let mut nearest = None;
            for &indices in &self.configuration.triangles {
                let vertices = indices.map(|index| self.vertices[index as usize]);
                let (weights, distance_squared) = closest_triangle_weights(point, vertices);
                if nearest
                    .is_none_or(|(_, _, nearest_distance)| distance_squared < nearest_distance)
                {
                    nearest = Some((indices, weights, distance_squared));
                }
            }
            let (indices, weights, _) =
                nearest.ok_or(PhysicsError::InvalidSoftBodyConfiguration {
                    field: "impulse topology",
                })?;
            (indices, weights)
        };
        let scaled_impulse = impulse.impulse * self.configuration.impulse_scale;
        for (vertex, weight) in triangle.into_iter().zip(weights) {
            let index = vertex as usize;
            self.velocities[index] += scaled_impulse * (self.inverse_masses[index] * weight);
        }
        self.awake = true;
        Ok(())
    }

    pub(crate) fn apply_volumetric_pressure(
        &mut self,
        pressure: SoftBodyPressure,
    ) -> Result<(), PhysicsError> {
        pressure.validate()?;
        let strength = pressure.strength * self.configuration.explosion_scale;
        let minimum_radius_squared = pressure.minimum_radius * pressure.minimum_radius;
        let mut total_impulse = 0.0;
        if self.configuration.maximum_collision_impulse > 0.0 {
            for &triangle in &self.configuration.triangles {
                if let Some(impulse) = pressure_impulse(
                    &self.vertices,
                    triangle,
                    pressure.epicenter,
                    strength,
                    minimum_radius_squared,
                ) {
                    total_impulse = impulse.length().mul_add(3.0, total_impulse);
                }
            }
        }
        let scale = if total_impulse > self.configuration.maximum_collision_impulse
            && self.configuration.maximum_collision_impulse > 0.0
        {
            self.configuration.maximum_collision_impulse / total_impulse
        } else {
            1.0
        };
        for &triangle in &self.configuration.triangles {
            let Some(impulse) = pressure_impulse(
                &self.vertices,
                triangle,
                pressure.epicenter,
                strength,
                minimum_radius_squared,
            ) else {
                continue;
            };
            for vertex in triangle {
                let index = vertex as usize;
                self.velocities[index] += impulse * (self.inverse_masses[index] * scale);
            }
        }
        self.awake = true;
        Ok(())
    }

    pub(crate) fn slice(
        &mut self,
        slice: SoftBodySlice,
    ) -> Result<Option<SoftBodySliceResult>, PhysicsError> {
        slice.validate()?;
        let Some(result) = crate::mesh_slice::slice_mesh(
            &self.vertices,
            &self.configuration.triangles,
            slice.triangle,
            SoftBodySlice::MINIMUM_EDGE_LENGTH,
            SoftBodySlice::MINIMUM_ISLAND_AREA_FRACTION,
        ) else {
            return Ok(None);
        };

        let source_configuration_velocities = self.configuration.velocities.clone();
        let source_configuration_targets = self.configuration.target_vertices.clone();
        let source_previous = self.previous_vertices.clone();
        let source_velocities = self.velocities.clone();
        let source_targets = self.target_vertices.clone();
        let inverse_entity_pose = self.entity_pose.inverse();
        let interpolate = |values: &[Vec3], source: [u32; 3], weights: [f32; 3]| {
            source
                .into_iter()
                .zip(weights)
                .map(|(index, weight)| values[index as usize] * weight)
                .sum::<Vec3>()
        };

        for vertex in &result.vertices {
            self.configuration
                .vertices
                .push(inverse_entity_pose.transform_point(vertex.position));
            if !source_configuration_velocities.is_empty() {
                self.configuration.velocities.push(interpolate(
                    &source_configuration_velocities,
                    vertex.source,
                    vertex.weights,
                ));
            }
            if !source_configuration_targets.is_empty() {
                self.configuration.target_vertices.push(interpolate(
                    &source_configuration_targets,
                    vertex.source,
                    vertex.weights,
                ));
            }
            self.vertices.push(vertex.position);
            self.previous_vertices.push(interpolate(
                &source_previous,
                vertex.source,
                vertex.weights,
            ));
            self.velocities.push(interpolate(
                &source_velocities,
                vertex.source,
                vertex.weights,
            ));
            self.target_vertices
                .push(interpolate(&source_targets, vertex.source, vertex.weights));
            self.normals.push(Vec3::ZERO);
            self.areas.push(0.0);
            self.inverse_masses.push(0.0);
            self.masses.push(0.0);
            self.levels.push(0);
            self.connected_to_attachment.push(false);
        }
        self.configuration.triangles = result.triangles;
        // A successful slice replaces the proxy geometry without carrying the
        // lattice. A lattice-backed body therefore continues as a surface soft
        // body, and the reconstruction removes its lattice-derived rigid core.
        self.configuration.tetrahedra.clear();
        self.configuration.rigid_core = None;
        self.configuration.flags = SoftBodyFlags::from_bits(
            self.configuration.flags.bits() & !SoftBodyFlags::RIGID_CORE.bits(),
        );
        self.rigid_core = None;
        (self.edges, self.fans) = soft_topology(
            &self.vertices,
            &self.configuration.triangles,
            self.configuration.flags,
        );
        self.recalculate_attachment_levels();
        self.recalculate_mass_distribution();
        self.recalculate_stretch_stiffness();
        self.recalculate_normals();
        self.awake = true;
        Ok(Some(SoftBodySliceResult {
            added_vertices: u32_from_usize(result.vertices.len()),
            removed_triangles: result.removed_triangles,
            added_triangles: result.added_triangles,
            removed_islands: result.removed_islands,
        }))
    }

    pub(crate) fn write_status(&self, output: &mut SoftBodyStatus) {
        output.vertices.clone_from(&self.vertices);
        output.velocities.clone_from(&self.velocities);
        output.normals.clone_from(&self.normals);
        output.vertex_map.clear();
        output
            .vertex_map
            .extend(0..u32_from_usize(self.vertices.len()));
        output.triangles.clone_from(&self.configuration.triangles);
        output.host_position = self.host_position;
        output.host_rotation = self.host_rotation;
        let (position, _) = deformable_bounds(&self.vertices);
        output.position = position;
        output.rotation = Quat::IDENTITY;
        output.awake = self.awake;
    }

    pub(crate) fn take_reactions(&mut self) -> SmallVec<[DeformableReaction; 16]> {
        core::mem::take(&mut self.reactions)
    }

    pub(crate) fn rigid_core_descriptor(&self) -> Option<SoftRigidCoreDescriptor> {
        let core = self.rigid_core.as_ref()?;
        let configuration = self.configuration.rigid_core.as_ref()?;
        let pose = PhysicsPose {
            translation: core.center,
            rotation: core.rotation,
        };
        let mut collider = configuration.collider.clone();
        collider.local_pose = pose.inverse() * (self.entity_pose * collider.local_pose);
        collider.mass = Some(core.mass);
        Some(SoftRigidCoreDescriptor { pose, collider })
    }

    pub(crate) fn bind_rigid_core(
        &mut self,
        body: RigidBodyHandle,
        collider: ColliderHandle,
    ) -> Result<(), PhysicsError> {
        let core = self
            .rigid_core
            .as_mut()
            .ok_or(PhysicsError::BackendInvariant(
                "soft rigid-core proxy was created without rigid-core state",
            ))?;
        core.native_body = Some(body);
        core.native_collider = Some(collider);
        Ok(())
    }

    pub(crate) fn rigid_core_handles(&self) -> Option<(RigidBodyHandle, ColliderHandle)> {
        let core = self.rigid_core.as_ref()?;
        Some((core.native_body?, core.native_collider?))
    }

    pub(crate) fn synchronize_rigid_core(
        &mut self,
        pose: PhysicsPose,
        has_contacts: bool,
    ) -> Result<(), PhysicsError> {
        let core = self
            .rigid_core
            .as_mut()
            .ok_or(PhysicsError::BackendInvariant(
                "soft rigid-core synchronization requires rigid-core state",
            ))?;
        core.center = pose.translation;
        core.rotation = pose.rotation;
        core.has_contacts = has_contacts;
        Ok(())
    }

    pub(crate) fn take_rigid_core_update(&mut self) -> Option<SoftRigidCoreUpdate> {
        let core = self.rigid_core.as_mut()?;
        Some(SoftRigidCoreUpdate {
            body: core.native_body?,
            collider: core.native_collider?,
            pose: PhysicsPose {
                translation: core.center,
                rotation: core.rotation,
            },
            mass: core.mass,
            fit_to_soft_body: !core.has_contacts,
            reactions: core::mem::take(&mut core.reactions),
        })
    }

    pub(crate) fn step<A, C, M>(
        &mut self,
        time_step: f32,
        mut attachment_frame: A,
        mut collide: C,
        mut medium_at: M,
    ) -> Result<(), PhysicsError>
    where
        A: FnMut(SoftBodyAttachment) -> Result<AttachmentFrame, PhysicsError>,
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
        M: FnMut(Vec3) -> MediumSample,
    {
        self.reactions.clear();
        let substeps = convert::substeps(time_step, self.configuration.maximum_time_step);
        let dt = time_step / f32_from_u32(substeps);
        for _ in 0..substeps {
            self.step_inner(dt, &mut attachment_frame, &mut collide, &mut medium_at)?;
        }
        let energy = self
            .velocities
            .iter()
            .map(|velocity| velocity.length_squared())
            .sum::<f32>()
            * (self.configuration.mass / f32_from_usize(self.velocities.len()));
        self.awake = energy > self.configuration.minimum_energy || !self.attachments.is_empty();
        Ok(())
    }

    fn step_inner<A, C, M>(
        &mut self,
        dt: f32,
        attachment_frame: &mut A,
        collide: &mut C,
        medium_at: &mut M,
    ) -> Result<(), PhysicsError>
    where
        A: FnMut(SoftBodyAttachment) -> Result<AttachmentFrame, PhysicsError>,
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
        M: FnMut(Vec3) -> MediumSample,
    {
        let previous_host_position = self.host_position;
        let previous_host_rotation = self.host_rotation;
        self.pin_attachments(attachment_frame)?;
        if self.host_initialized && self.configuration.host_space_simulation > 0.0 {
            let inverse_previous_rotation = previous_host_rotation.inverse();
            let rigid_fraction = self.configuration.host_space_simulation;
            for index in 0..self.vertices.len() {
                if self.inverse_masses[index] == 0.0 || !self.connected_to_attachment[index] {
                    continue;
                }
                let local =
                    inverse_previous_rotation * (self.vertices[index] - previous_host_position);
                let rigid_position = self.host_position + self.host_rotation * local;
                self.vertices[index] = self.vertices[index].lerp(rigid_position, rigid_fraction);
            }
        }
        self.host_initialized = !self.attachments.is_empty();
        let force_acceleration = self.pending_force / self.configuration.mass;
        self.pending_force = Vec3::ZERO;
        let torque = core::mem::take(&mut self.pending_torque);
        if torque != Vec3::ZERO {
            self.apply_angular_impulse(torque * dt);
        }
        self.advance_rigid_core();
        if !self.configuration.tetrahedra.is_empty() {
            self.stabilize_tetrahedra(dt);
        }
        self.limit_animation_distance(dt);
        self.previous_vertices.clone_from(&self.vertices);
        for index in 0..self.vertices.len() {
            if self.inverse_masses[index] == 0.0 {
                continue;
            }
            self.velocities[index] += force_acceleration * dt;
            self.vertices[index] += self.velocities[index] * dt;
        }
        self.enforce_maximum_stretch();
        self.pin_attachments(attachment_frame)?;
        self.enforce_maximum_stretch();

        self.resolve_vertex_contacts(dt, collide)?;

        self.advance_wind(dt);
        self.recalculate_normals();
        self.apply_medium_forces(dt, medium_at);
        self.apply_shape_stiffness(dt);
        self.apply_animation_velocity(dt);
        if self.configuration.tetrahedra.is_empty() {
            self.solve_surface_edges();
        } else {
            self.solve_volumetric_edges(dt);
        }
        self.fit_rigid_core();
        let damping = (self.configuration.damping * dt).clamp(0.0, 1.0);
        for index in 0..self.vertices.len() {
            let host_velocity = self.host_linear_velocity
                + self
                    .host_angular_velocity
                    .cross(self.vertices[index] - self.host_position);
            let retained = 1.0
                - damping
                    * if self.attachments.contains_key(&u32_from_usize(index)) {
                        2.0
                    } else {
                        1.0
                    };
            self.velocities[index] =
                self.velocities[index] * retained + host_velocity * (1.0 - retained);
        }
        Ok(())
    }

    /// Sweeps every vertex against the world, projects it out of its contact,
    /// and records the equal-and-opposite reaction on a dynamic body.
    fn resolve_vertex_contacts<C>(&mut self, dt: f32, collide: &mut C) -> Result<(), PhysicsError>
    where
        C: FnMut(Vec3, Vec3, f32) -> Result<Option<DeformableContact>, PhysicsError>,
    {
        for index in 0..self.vertices.len() {
            let Some(contact) = collide(
                self.previous_vertices[index],
                self.vertices[index],
                self.configuration.thickness,
            )?
            else {
                if self.inverse_masses[index] > 0.0 {
                    self.velocities[index] =
                        (self.vertices[index] - self.previous_vertices[index]) / dt;
                }
                continue;
            };
            let free_velocity = (self.vertices[index] - self.previous_vertices[index]) / dt;
            self.vertices[index] = contact.position + contact.normal * self.configuration.thickness;
            let velocity = (self.vertices[index] - self.previous_vertices[index]) / dt;
            let contact_velocity = contact.velocity
                - (self.host_linear_velocity
                    + self
                        .host_angular_velocity
                        .cross(contact.position - self.host_center))
                    * self.configuration.host_space_simulation;
            let residual = contact.normal.dot(velocity - contact_velocity);
            let mut velocity = velocity - contact.normal * residual.min(0.0);
            if self.configuration.friction * residual < 0.0 {
                let mut tangent = velocity - contact_velocity;
                tangent -= contact.normal * contact.normal.dot(tangent);
                let friction_limit = -self.configuration.friction * residual;
                if tangent.length_squared() > friction_limit * friction_limit {
                    velocity -= tangent.normalize_or_zero() * friction_limit;
                } else {
                    velocity = contact_velocity;
                }
            }
            if contact.dynamic && self.inverse_masses[index] > 0.0 {
                let vertex_impulse = (velocity - free_velocity) / self.inverse_masses[index];
                if vertex_impulse.is_finite() && vertex_impulse != Vec3::ZERO {
                    self.reactions.push(DeformableReaction {
                        body: contact.body,
                        point: contact.position,
                        impulse: -vertex_impulse,
                    });
                }
            }
            self.velocities[index] = velocity;
        }
        Ok(())
    }

    /// Applies gravity, buoyancy, and the per-vertex aerodynamic normal force
    /// for the medium each vertex currently sits in.
    fn apply_medium_forces<M>(&mut self, dt: f32, medium_at: &mut M)
    where
        M: FnMut(Vec3) -> MediumSample,
    {
        let area_sum = self.areas.iter().sum::<f32>();
        let area_scale = if area_sum > 0.0 {
            f32_from_usize(self.vertices.len()) / area_sum
        } else {
            0.0
        };
        let vertex_volume = self.configuration.mass
            / (f32_from_usize(self.vertices.len()) * self.configuration.density);
        let air_wind =
            self.wind_previous * self.wind_timer + self.wind_next * (1.0 - self.wind_timer);
        for index in 0..self.vertices.len() {
            if self.inverse_masses[index] == 0.0 {
                continue;
            }
            let medium = medium_at(self.vertices[index]);
            let gravity = medium.gravity.unwrap_or(self.configuration.gravity);
            let (resistance, flow) = if medium.submerged_depth > 0.0 {
                let depth_fraction =
                    (medium.submerged_depth / self.configuration.thickness).min(1.0);
                self.velocities[index] -=
                    gravity * (medium.water_density * vertex_volume * depth_fraction * dt);
                (self.configuration.water_resistance, medium.velocity)
            } else {
                (
                    self.configuration.air_resistance,
                    air_wind + medium.velocity,
                )
            };
            let relative_flow = flow - self.velocities[index];
            let normal_force = self.normals[index]
                * (self.normals[index].dot(relative_flow)
                    * self.areas[index]
                    * area_scale
                    * resistance
                    * dt);
            self.velocities[index] += gravity * dt + normal_force;
        }
    }

    /// Cry `m_maxSafeStep` is a fractional edge-stretch guard, not a time-step
    /// limit. Vertices are processed in attachment-distance order in the
    /// native solver; `levels` provides the same stable choice here.
    fn enforce_maximum_stretch(&mut self) {
        let maximum_safe_step = self.configuration.maximum_safe_step;
        if maximum_safe_step <= 0.0 {
            return;
        }
        for edge in &self.edges {
            let [a, b] = edge.vertices;
            if self.configuration.mass_decay > 0.0
                && (self.masses[a] - self.masses[b]).abs() <= f32::EPSILON
            {
                continue;
            }
            let maximum_length = edge.rest_length * (1.0 + maximum_safe_step);
            let delta = self.vertices[b] - self.vertices[a];
            if delta.length_squared() <= maximum_length * maximum_length {
                continue;
            }
            let a_movable = self.inverse_masses[a] > 0.0;
            let b_movable = self.inverse_masses[b] > 0.0;
            let moving = if a_movable && (!b_movable || self.levels[a] > self.levels[b]) {
                a
            } else if b_movable {
                b
            } else {
                continue;
            };
            let anchor = if moving == a { b } else { a };
            let direction = (self.vertices[moving] - self.vertices[anchor]).normalize_or_zero();
            self.vertices[moving] = self.vertices[anchor] + direction * maximum_length;
        }
    }

    fn pin_attachments<A>(&mut self, attachment_frame: &mut A) -> Result<(), PhysicsError>
    where
        A: FnMut(SoftBodyAttachment) -> Result<AttachmentFrame, PhysicsError>,
    {
        let mut host_frames = BTreeMap::new();
        for attachment in self.attachments.values().copied() {
            let frame = attachment_frame(attachment)?;
            host_frames.entry(attachment.body).or_insert(frame);
            let index = attachment.vertex as usize;
            self.vertices[index] = frame.position;
            self.velocities[index] =
                frame.velocity * (1.0 - self.configuration.host_space_simulation);
        }
        if let Some(frame) = host_frames.values().next_back().copied() {
            self.host_position = frame.body_position;
            self.host_rotation = frame.rotation;
        }
        let reciprocal_host_count = if host_frames.is_empty() {
            0.0
        } else {
            1.0 / f32_from_usize(host_frames.len())
        };
        self.host_center =
            host_frames.values().map(|frame| frame.center).sum::<Vec3>() * reciprocal_host_count;
        self.host_linear_velocity = host_frames
            .values()
            .map(|frame| frame.linear_velocity)
            .sum::<Vec3>()
            * reciprocal_host_count;
        self.host_angular_velocity = host_frames
            .values()
            .map(|frame| frame.angular_velocity)
            .sum::<Vec3>()
            * reciprocal_host_count;
        Ok(())
    }

    fn limit_animation_distance(&mut self, dt: f32) {
        if self.configuration.animation_stiffness == 0.0 {
            return;
        }
        for index in 0..self.vertices.len() {
            if self.inverse_masses[index] == 0.0 {
                continue;
            }
            let decay = self.configuration.animation_stiffness_decay.mul_add(
                -f32_from_u32(self.levels[index]).mul_add(-self.maximum_level_reciprocal, 1.0),
                1.0,
            );
            let target = self.host_position + self.host_rotation * self.target_vertices[index];
            let mut delta = target - self.vertices[index];
            let maximum_distance = if self.configuration.maximum_animation_distance > 0.0 {
                self.configuration.maximum_animation_distance
            } else {
                1.0e5
            } * decay;
            if delta.length_squared() > maximum_distance * maximum_distance {
                self.vertices[index] = target - delta.normalize_or_zero() * maximum_distance;
                delta = target - self.vertices[index];
            }
            let velocity_squared = self.velocities[index].length_squared();
            if velocity_squared > f32::EPSILON
                && delta.dot(delta + self.velocities[index] * dt) < 0.0
            {
                self.vertices[index] += self.velocities[index]
                    * (self.velocities[index].dot(delta) / velocity_squared - dt);
            }
        }
    }

    fn apply_animation_velocity(&mut self, dt: f32) {
        if self.configuration.animation_stiffness == 0.0 {
            return;
        }
        let retained = self
            .configuration
            .animation_damping
            .mul_add(-dt, 1.0)
            .max(0.0);
        for index in 0..self.vertices.len() {
            if self.inverse_masses[index] == 0.0 {
                continue;
            }
            let target = self.host_position + self.host_rotation * self.target_vertices[index];
            let requested_velocity = (target - self.vertices[index])
                * self.configuration.animation_stiffness
                * (self.configuration.animation_stiffness_decay * f32_from_u32(self.levels[index]))
                    .mul_add(-self.maximum_level_reciprocal, 1.0);
            self.velocities[index] =
                self.velocities[index] * retained + requested_velocity * (1.0 - retained);
        }
    }

    fn apply_shape_stiffness(&mut self, dt: f32) {
        if self.configuration.normal_shape_stiffness + self.configuration.tangential_shape_stiffness
            <= 0.0
        {
            return;
        }
        for fan in &self.fans {
            let center = fan.vertex;
            if self.inverse_masses[center] == 0.0 || fan.neighbors.is_empty() {
                continue;
            }
            let normal = self.normals[center];
            let mut normal_angle = 0.0;
            for neighbor_index in 0..fan.neighbors.len() {
                let next_index = if neighbor_index + 1 < fan.neighbors.len() {
                    neighbor_index + 1
                } else {
                    0
                };
                let neighbor = fan.neighbors[neighbor_index];
                let next = fan.neighbors[next_index];
                let edge = self.vertices[neighbor] - self.vertices[center];
                let next_edge = self.vertices[next] - self.vertices[center];
                let edge_length = edge.length().max(1.0e-4);
                let next_edge_length = next_edge.length().max(1.0e-4);
                normal_angle += 1.0 - normal.dot(edge) / edge_length;

                let mut axis = edge.cross(next_edge);
                let axis_length = axis.length();
                if axis_length > f32::EPSILON {
                    let sign = if next_edge.dot(edge) < 0.0 { -1.0 } else { 1.0 };
                    axis *= (fan.rest_pair_angles[neighbor_index] - 1.0 + sign) / axis_length
                        - sign / (edge_length * next_edge_length);
                    axis *= dt * self.configuration.tangential_shape_stiffness;
                    self.velocities[neighbor] -= axis.cross(edge);
                    self.velocities[next] += axis.cross(next_edge);
                }
            }
            for &neighbor in &fan.neighbors {
                let edge = self.vertices[neighbor] - self.vertices[center];
                let reciprocal_length = 1.0 / edge.length().max(1.0e-4);
                let mut correction =
                    (normal * edge.length_squared() - edge * normal.dot(edge)) * reciprocal_length;
                correction *= (normal_angle - fan.rest_normal_angle)
                    * dt
                    * self.configuration.normal_shape_stiffness;
                self.velocities[neighbor] += correction;
                self.velocities[center] -= correction;
            }
        }
    }

    fn solve_surface_edges(&mut self) {
        for _ in 0..self.configuration.maximum_iterations {
            let mut maximum_residual = 0.0_f32;
            for edge in &self.edges {
                let [a, b] = edge.vertices;
                let delta = self.vertices[b] - self.vertices[a];
                let length = delta.length();
                if length <= 1.0e-4 {
                    continue;
                }
                let direction = delta / length;
                let target_separation_velocity =
                    (length - edge.rest_length) * self.stretch_stiffness;
                let residual = (self.velocities[a] - self.velocities[b]).dot(direction)
                    - target_separation_velocity;
                let inverse_mass_sum = self.inverse_masses[a] + self.inverse_masses[b];
                if inverse_mass_sum <= 1.0e-10 {
                    continue;
                }
                let impulse = direction * -residual.min(100.0);
                let effective_mass = inverse_mass_sum.recip();
                self.velocities[a] += impulse * (self.inverse_masses[a] * effective_mass);
                self.velocities[b] -= impulse * (self.inverse_masses[b] * effective_mass);
                maximum_residual = maximum_residual.max(-residual);
            }
            if maximum_residual <= self.configuration.accuracy {
                break;
            }
        }
    }

    fn initialize_rigid_core(&mut self) {
        if !self.configuration.flags.contains(SoftBodyFlags::RIGID_CORE) {
            self.rigid_core = None;
            return;
        }
        let Some(configuration) = self.configuration.rigid_core.as_ref() else {
            self.rigid_core = None;
            return;
        };
        let indices = configuration
            .contained_vertices
            .iter()
            .map(|&index| index as usize)
            .collect::<Vec<_>>();
        let mass = indices.iter().map(|&index| self.masses[index]).sum::<f32>();
        if mass <= 0.0 {
            self.rigid_core = None;
            return;
        }
        let center = indices
            .iter()
            .map(|&index| self.vertices[index] * self.masses[index])
            .sum::<Vec3>()
            / mass;
        let inverse_rotation = self.entity_pose.rotation.inverse();
        let local_offsets = indices
            .iter()
            .map(|&index| inverse_rotation * (self.vertices[index] - center))
            .collect::<Vec<_>>();
        let native = self
            .rigid_core
            .as_ref()
            .map(|core| (core.native_body, core.native_collider));
        self.rigid_core = Some(SoftRigidCore {
            indices,
            local_offsets,
            center,
            rotation: self.entity_pose.rotation,
            mass,
            native_body: native.and_then(|handles| handles.0),
            native_collider: native.and_then(|handles| handles.1),
            has_contacts: false,
            reactions: SmallVec::new(),
        });
    }

    fn advance_rigid_core(&mut self) {
        let Some(core) = &mut self.rigid_core else {
            return;
        };
        core.reactions.clear();
        if !core.has_contacts {
            return;
        }
        for (&index, &local_offset) in core.indices.iter().zip(&core.local_offsets) {
            let desired_position = core.center + core.rotation * local_offset;
            let requested_velocity = (desired_position - self.vertices[index]) * 5.0;
            let direction = requested_velocity.normalize_or_zero();
            let delta_velocity = direction
                * direction
                    .dot(requested_velocity - self.velocities[index])
                    .max(0.0);
            self.velocities[index] += delta_velocity;
            let impulse = -delta_velocity * self.masses[index];
            if impulse.is_finite() && impulse != Vec3::ZERO {
                core.reactions.push(SoftRigidCoreReaction {
                    point: desired_position,
                    impulse,
                });
            }
        }
    }

    fn fit_rigid_core(&mut self) {
        let Some(core) = &mut self.rigid_core else {
            return;
        };
        if core.has_contacts {
            return;
        }
        let center = core
            .indices
            .iter()
            .map(|&index| self.vertices[index] * self.masses[index])
            .sum::<Vec3>()
            / core.mass;
        let mut axis = Vec3::ZERO;
        for (&index, &local_offset) in core.indices.iter().zip(&core.local_offsets) {
            axis += (core.rotation * local_offset).cross(self.vertices[index] - center)
                * self.masses[index];
        }
        axis = axis.normalize_or_zero();
        if axis != Vec3::ZERO {
            let mut sine = 0.0;
            let mut cosine = 0.0;
            for (&index, &local_offset) in core.indices.iter().zip(&core.local_offsets) {
                let core_offset = core.rotation * local_offset;
                let axial = axis * axis.dot(core_offset);
                let target_offset = self.vertices[index] - center - axial;
                sine += axis.cross(core_offset).dot(target_offset);
                cosine += (core_offset - axial).dot(target_offset);
            }
            core.rotation =
                (Quat::from_axis_angle(axis, sine.atan2(cosine)) * core.rotation).normalize();
        }
        core.center = center;
    }

    fn stabilize_tetrahedra(&mut self, dt: f32) {
        for _ in 0..self.configuration.maximum_iterations {
            let mut corrected = false;
            for tetrahedron in &self.configuration.tetrahedra {
                let indices = tetrahedron.map(|index| index as usize);
                let current = indices.map(|index| self.vertices[index]);
                let predicted =
                    indices.map(|index| self.vertices[index] + self.velocities[index] * dt);
                let current_volume = signed_tetrahedron_volume(current);
                let predicted_volume = signed_tetrahedron_volume(predicted);
                if current_volume * predicted_volume < 0.0 {
                    project_tetrahedron_rigid_velocity(
                        indices,
                        &self.vertices,
                        &self.masses,
                        &mut self.velocities,
                    );
                    corrected = true;
                }
            }
            if !corrected {
                break;
            }
        }
    }

    fn solve_volumetric_edges(&mut self, dt: f32) {
        for edge in &self.edges {
            let [a, b] = edge.vertices;
            let delta = self.vertices[b] - self.vertices[a];
            let length = delta.length();
            if length <= 1.0e-4 {
                continue;
            }
            let impulse =
                delta * (self.stretch_stiffness * (length - edge.rest_length) * dt / length);
            self.velocities[a] += impulse * self.inverse_masses[a];
            self.velocities[b] -= impulse * self.inverse_masses[b];
        }
        for edge in &self.edges {
            let [a, b] = edge.vertices;
            let delta = self.vertices[b] - self.vertices[a];
            let length_squared = delta.length_squared();
            if length_squared <= 1.0e-8 {
                continue;
            }
            let relative_velocity = self.velocities[b] - self.velocities[a];
            let impulse = delta
                * (relative_velocity.dot(delta) * self.stretch_stiffness * dt.powi(2) * 0.5
                    / length_squared);
            self.velocities[a] += impulse * self.inverse_masses[a];
            self.velocities[b] -= impulse * self.inverse_masses[b];
        }
        for _ in 0..self.configuration.maximum_iterations {
            let mut corrected = false;
            for edge in &self.edges {
                let [a, b] = edge.vertices;
                let direction = self.vertices[b] - self.vertices[a];
                let current_length = direction.length();
                if (current_length - edge.rest_length).abs() <= edge.rest_length * 0.1 {
                    continue;
                }
                let relative_velocity = self.velocities[b] - self.velocities[a];
                let predicted = direction + relative_velocity * dt;
                let rest_length_squared = edge.rest_length.powi(2);
                let current_error = direction.length_squared() - rest_length_squared;
                let predicted_error = predicted.length_squared() - rest_length_squared;
                if current_error * predicted_error >= 0.0
                    || predicted_error.abs() <= current_error.abs()
                {
                    continue;
                }
                let inverse_mass_sum = self.inverse_masses[a] + self.inverse_masses[b];
                let axis = direction * (inverse_mass_sum * dt);
                let quadratic = axis.length_squared();
                if quadratic <= f32::EPSILON {
                    continue;
                }
                let linear = predicted.dot(axis);
                let discriminant = quadratic.mul_add(
                    -(predicted.length_squared() - rest_length_squared),
                    linear * linear,
                );
                if discriminant > 0.0 {
                    let impulse = direction
                        * (signed_nonzero(relative_velocity.dot(direction))
                            .mul_add(-discriminant.sqrt(), linear)
                            / quadratic);
                    self.velocities[a] += impulse * self.inverse_masses[a];
                    self.velocities[b] -= impulse * self.inverse_masses[b];
                    corrected = true;
                }
            }
            if !corrected {
                break;
            }
        }
        for velocity in &mut self.velocities {
            if velocity.length_squared() > 400.0 {
                *velocity = velocity.normalize_or_zero() * 20.0;
            }
        }
    }

    fn recalculate_normals(&mut self) {
        self.normals.fill(Vec3::ZERO);
        self.areas.fill(0.0);
        let coverage = if self
            .configuration
            .flags
            .contains(SoftBodyFlags::SKIP_LONGEST_EDGES)
        {
            0.5
        } else {
            1.0 / 3.0
        };
        for fan in &self.fans {
            let center = fan.vertex;
            let pair_count = if fan.full {
                fan.neighbors.len()
            } else {
                fan.neighbors.len().saturating_sub(1)
            };
            for index in 0..pair_count {
                let next = (index + 1) % fan.neighbors.len();
                let edge = self.vertices[fan.neighbors[index]] - self.vertices[center];
                let next_edge = self.vertices[fan.neighbors[next]] - self.vertices[center];
                self.normals[center] += edge.cross(next_edge);
            }
            let double_area = self.normals[center].length();
            if double_area > 0.0 {
                self.normals[center] /= double_area;
                self.areas[center] = double_area * coverage * 0.5;
            }
        }
        if self.fans.is_empty() {
            for triangle in &self.configuration.triangles {
                let [a, b, c] = triangle.map(|index| index as usize);
                let normal = (self.vertices[b] - self.vertices[a])
                    .cross(self.vertices[c] - self.vertices[a]);
                self.normals[a] += normal;
                self.normals[b] += normal;
                self.normals[c] += normal;
            }
            for normal in &mut self.normals {
                *normal = normal.normalize_or_zero();
            }
        }
    }

    fn advance_wind(&mut self, dt: f32) {
        self.wind_timer = dt.mul_add(4.0, self.wind_timer);
        if self.wind_timer <= 1.0 {
            return;
        }
        self.wind_timer = 0.0;
        self.wind_previous = self.wind_next;
        let variation = self.configuration.wind_variance
            * (self.configuration.wind.x.abs()
                + self.configuration.wind.y.abs()
                + self.configuration.wind.z.abs());
        self.wind_next = self.configuration.wind
            + Vec3::new(
                deterministic_fraction(&mut self.wind_seed),
                deterministic_fraction(&mut self.wind_seed),
                deterministic_fraction(&mut self.wind_seed),
            ) * variation;
    }

    fn recalculate_attachment_levels(&mut self) {
        (self.levels, self.connected_to_attachment) = soft_levels(
            self.vertices.len(),
            &self.edges,
            self.attachments.keys().copied(),
        );
        let maximum_level = self.levels.iter().copied().max().unwrap_or(0);
        self.maximum_level_reciprocal = 1.0 / f32_from_u32(maximum_level.max(1));
    }

    fn recalculate_mass_distribution(&mut self) {
        let base_inverse_mass = f32_from_usize(self.vertices.len()) / self.configuration.mass;
        let maximum_level = self.levels.iter().copied().max().unwrap_or(0);
        let mass_gradient =
            (self.configuration.mass_decay + 1.0) / f32_from_u32(maximum_level.saturating_add(1));
        for index in 0..self.inverse_masses.len() {
            let dynamic_inverse_mass = if self.configuration.mass_decay > 0.0 {
                base_inverse_mass * f32_from_u32(self.levels[index]).mul_add(mass_gradient, 1.0)
            } else {
                base_inverse_mass
            };
            self.masses[index] = dynamic_inverse_mass.recip();
            self.inverse_masses[index] = if self.attachments.contains_key(&u32_from_usize(index)) {
                0.0
            } else {
                dynamic_inverse_mass
            };
        }
    }

    fn recalculate_stretch_stiffness(&mut self) {
        self.stretch_stiffness = if self.configuration.stiffness < 0.0 {
            -self.configuration.stiffness * self.configuration.mass
                / (f32_from_usize(self.vertices.len())
                    * self.configuration.maximum_time_step.powi(2))
        } else {
            self.configuration.stiffness
        };
    }
}

pub fn deformable_bounds(points: &[Vec3]) -> (Vec3, f32) {
    let mut minimum = Vec3::splat(f32::MAX);
    let mut maximum = Vec3::splat(f32::MIN);
    for point in points {
        minimum = minimum.min(*point);
        maximum = maximum.max(*point);
    }
    let center = (minimum + maximum) * 0.5;
    let radius = points
        .iter()
        .map(|point| point.distance(center))
        .fold(1.0e-4, f32::max);
    (center, radius)
}

fn soft_topology(
    vertices: &[Vec3],
    triangles: &[[u32; 3]],
    flags: SoftBodyFlags,
) -> (Vec<SoftEdge>, Vec<SoftFan>) {
    let (edge_pairs, mesh_edge_counts) = soft_edge_pairs(vertices, triangles, flags);
    let edges = edge_pairs
        .iter()
        .copied()
        .map(|(a, b)| SoftEdge {
            vertices: [a, b],
            rest_length: vertices[a].distance(vertices[b]),
        })
        .collect::<Vec<_>>();

    let fans = (0..vertices.len())
        .map(|vertex| soft_fan(vertex, vertices, triangles, &edge_pairs, &mesh_edge_counts))
        .collect();
    (edges, fans)
}

/// A surface soft body's unique spring edges, paired with how many triangles
/// each mesh edge belongs to.
type SoftEdgeTopology = (BTreeSet<(usize, usize)>, BTreeMap<(usize, usize), u32>);

/// The unique spring edges of a surface soft body, plus how many triangles each
/// mesh edge belongs to. `SKIP_LONGEST_EDGES` drops each triangle's longest
/// edge from the spring set but still counts it for the manifold test.
fn soft_edge_pairs(
    vertices: &[Vec3],
    triangles: &[[u32; 3]],
    flags: SoftBodyFlags,
) -> SoftEdgeTopology {
    let mut edges = BTreeSet::new();
    let mut mesh_edge_counts = BTreeMap::<(usize, usize), u32>::new();
    for triangle in triangles {
        let indices = triangle.map(|index| index as usize);
        let mut triangle_edges = [
            (indices[0].min(indices[1]), indices[0].max(indices[1])),
            (indices[1].min(indices[2]), indices[1].max(indices[2])),
            (indices[2].min(indices[0]), indices[2].max(indices[0])),
        ];
        for edge in triangle_edges {
            *mesh_edge_counts.entry(edge).or_default() += 1;
        }
        if flags.contains(SoftBodyFlags::SKIP_LONGEST_EDGES) {
            let longest = triangle_edges
                .iter()
                .enumerate()
                .max_by(|(_, left), (_, right)| {
                    vertices[left.0]
                        .distance_squared(vertices[left.1])
                        .total_cmp(&vertices[right.0].distance_squared(vertices[right.1]))
                })
                .map_or(0, |(index, _)| index);
            triangle_edges[longest] = (usize::MAX, usize::MAX);
        }
        for edge in triangle_edges {
            if edge.0 != usize::MAX {
                edges.insert(edge);
            }
        }
    }
    (edges, mesh_edge_counts)
}

/// One vertex's neighbour fan, wound counter-clockwise about the vertex normal
/// and cut at its widest gap when the fan is not closed.
fn soft_fan(
    vertex: usize,
    vertices: &[Vec3],
    triangles: &[[u32; 3]],
    edge_pairs: &BTreeSet<(usize, usize)>,
    mesh_edge_counts: &BTreeMap<(usize, usize), u32>,
) -> SoftFan {
    let mut neighbors = edge_pairs
        .iter()
        .filter_map(|&(a, b)| {
            if a == vertex {
                Some(b)
            } else if b == vertex {
                Some(a)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    let mut normal = Vec3::ZERO;
    for triangle in triangles {
        let [a, b, c] = triangle.map(|index| index as usize);
        if a == vertex || b == vertex || c == vertex {
            normal += (vertices[b] - vertices[a]).cross(vertices[c] - vertices[a]);
        }
    }
    normal = normal.normalize_or(Vec3::Z);
    let tangent = orthogonal(normal);
    let bitangent = normal.cross(tangent);
    let full = mesh_edge_counts
        .iter()
        .filter(|((a, b), _)| *a == vertex || *b == vertex)
        .all(|(_, &count)| count == 2);
    let angle = |neighbor: usize| {
        let direction = (vertices[neighbor] - vertices[vertex]).normalize_or_zero();
        direction.dot(bitangent).atan2(direction.dot(tangent))
    };
    neighbors.sort_by(|&left, &right| angle(left).total_cmp(&angle(right)));
    if !full && neighbors.len() > 1 {
        let mut largest_gap = f32::MIN;
        let mut start = 0;
        for index in 0..neighbors.len() {
            let next = (index + 1) % neighbors.len();
            let gap = (angle(neighbors[next]) - angle(neighbors[index]))
                .rem_euclid(core::f32::consts::TAU);
            if gap > largest_gap {
                largest_gap = gap;
                start = next;
            }
        }
        neighbors.rotate_left(start);
    }
    let mut rest_pair_angles = Vec::with_capacity(neighbors.len());
    let mut rest_normal_angle = 0.0;
    for index in 0..neighbors.len() {
        let next = (index + 1) % neighbors.len();
        let edge = (vertices[neighbors[index]] - vertices[vertex]).normalize_or_zero();
        let next_edge = (vertices[neighbors[next]] - vertices[vertex]).normalize_or_zero();
        let sign = if next_edge.dot(edge) < 0.0 { -1.0 } else { 1.0 };
        rest_pair_angles.push(edge.cross(next_edge).length().mul_add(sign, 1.0) - sign);
        rest_normal_angle += 1.0 - normal.dot(edge);
    }
    SoftFan {
        vertex,
        neighbors,
        full,
        rest_pair_angles,
        rest_normal_angle,
    }
}

fn soft_lattice_edges(vertices: &[Vec3], tetrahedra: &[[u32; 4]]) -> Vec<SoftEdge> {
    let mut pairs = BTreeSet::new();
    for tetrahedron in tetrahedra {
        let indices = tetrahedron.map(|index| index as usize);
        for a in 0..3 {
            for b in (a + 1)..4 {
                pairs.insert((indices[a].min(indices[b]), indices[a].max(indices[b])));
            }
        }
    }
    pairs
        .into_iter()
        .map(|(a, b)| SoftEdge {
            vertices: [a, b],
            rest_length: vertices[a].distance(vertices[b]),
        })
        .collect()
}

fn soft_levels(
    vertex_count: usize,
    edges: &[SoftEdge],
    attached_vertices: impl IntoIterator<Item = u32>,
) -> (Vec<u32>, Vec<bool>) {
    let mut adjacent = vec![Vec::new(); vertex_count];
    for edge in edges {
        let [a, b] = edge.vertices;
        adjacent[a].push(b);
        adjacent[b].push(a);
    }

    let mut levels = vec![u32::MAX; vertex_count];
    let mut queue = VecDeque::new();
    for vertex in attached_vertices {
        let vertex = vertex as usize;
        if vertex < vertex_count && levels[vertex] == u32::MAX {
            levels[vertex] = 0;
            queue.push_back(vertex);
        }
    }
    while let Some(vertex) = queue.pop_front() {
        let next_level = levels[vertex].saturating_add(1);
        for &neighbor in &adjacent[vertex] {
            if levels[neighbor] == u32::MAX {
                levels[neighbor] = next_level;
                queue.push_back(neighbor);
            }
        }
    }
    let connected = levels.iter().map(|&level| level != u32::MAX).collect();
    for level in &mut levels {
        *level = if *level == u32::MAX { 0 } else { *level };
    }
    (levels, connected)
}

fn signed_tetrahedron_volume([a, b, c, d]: [Vec3; 4]) -> f32 {
    (b - a).cross(c - a).dot(d - a)
}

fn project_tetrahedron_rigid_velocity(
    indices: [usize; 4],
    positions: &[Vec3],
    masses: &[f32],
    velocities: &mut [Vec3],
) {
    let total_mass = indices.iter().map(|&index| masses[index]).sum::<f32>();
    if total_mass <= 0.0 {
        return;
    }
    let center = indices
        .iter()
        .map(|&index| positions[index] * masses[index])
        .sum::<Vec3>()
        / total_mass;
    let linear_momentum = indices
        .iter()
        .map(|&index| velocities[index] * masses[index])
        .sum::<Vec3>();
    let mut angular_momentum = Vec3::ZERO;
    let mut inertia = Mat3::ZERO;
    for &index in &indices {
        let offset = positions[index] - center;
        let momentum = velocities[index] * masses[index];
        angular_momentum += offset.cross(momentum);
        let outer = Mat3::from_cols(offset * offset.x, offset * offset.y, offset * offset.z);
        inertia += (Mat3::IDENTITY * offset.length_squared() - outer) * masses[index];
    }
    let angular_velocity = if inertia.determinant().abs() > 1.0e-10 {
        inertia.inverse() * angular_momentum
    } else {
        Vec3::ZERO
    };
    let linear_velocity = linear_momentum / total_mass;
    for &index in &indices {
        velocities[index] = linear_velocity + angular_velocity.cross(positions[index] - center);
    }
}

fn triangle_barycentric_weights(point: Vec3, [a, b, c]: [Vec3; 3]) -> [f32; 3] {
    let double_area = (b - a).cross(c - a).length();
    if double_area <= 1.0e-4 {
        return [1.0, 0.0, 0.0];
    }
    [
        (b - point).cross(c - point).length() / double_area,
        (c - point).cross(a - point).length() / double_area,
        (a - point).cross(b - point).length() / double_area,
    ]
}

fn pressure_impulse(
    vertices: &[Vec3],
    triangle: [u32; 3],
    epicenter: Vec3,
    strength: f32,
    minimum_radius_squared: f32,
) -> Option<Vec3> {
    let [a, b, c] = triangle.map(|index| vertices[index as usize]);
    let center = (a + b + c) / 3.0;
    let radial = center - epicenter;
    let radius_squared = radial.length_squared();
    let normal = (c - a).cross(b - a);
    let normal_squared = normal.length_squared();
    let facing = normal.dot(radial);
    if facing < 0.0 || radius_squared <= f32::EPSILON || normal_squared <= f32::EPSILON {
        return None;
    }
    let denominator = (normal_squared * radius_squared).sqrt()
        * minimum_radius_squared.max(radius_squared).max(f32::EPSILON);
    Some(normal * (facing * 0.5 * strength / denominator))
}

fn closest_triangle_weights(point: Vec3, [first, second, third]: [Vec3; 3]) -> ([f32; 3], f32) {
    let ab = second - first;
    let ac = third - first;
    let ap = point - first;
    let d1 = ab.dot(ap);
    let d2 = ac.dot(ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return ([1.0, 0.0, 0.0], point.distance_squared(first));
    }

    let bp = point - second;
    let d3 = ab.dot(bp);
    let d4 = ac.dot(bp);
    if d3 >= 0.0 && d4 <= d3 {
        return ([0.0, 1.0, 0.0], point.distance_squared(second));
    }

    let vc = d3.mul_add(-d2, d1 * d4);
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let beta = d1 / (d1 - d3);
        let closest = first + ab * beta;
        return ([1.0 - beta, beta, 0.0], point.distance_squared(closest));
    }

    let cp = point - third;
    let d5 = ab.dot(cp);
    let d6 = ac.dot(cp);
    if d6 >= 0.0 && d5 <= d6 {
        return ([0.0, 0.0, 1.0], point.distance_squared(third));
    }

    let vb = d1.mul_add(-d6, d5 * d2);
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let gamma = d2 / (d2 - d6);
        let closest = first + ac * gamma;
        return ([1.0 - gamma, 0.0, gamma], point.distance_squared(closest));
    }

    let va = d5.mul_add(-d4, d3 * d6);
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let gamma = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let closest = second + (third - second) * gamma;
        return ([0.0, 1.0 - gamma, gamma], point.distance_squared(closest));
    }

    let reciprocal = 1.0 / (va + vb + vc);
    let beta = vb * reciprocal;
    let gamma = vc * reciprocal;
    let closest = first + ab * beta + ac * gamma;
    (
        [1.0 - beta - gamma, beta, gamma],
        point.distance_squared(closest),
    )
}

fn nearest_segment(points: &[Vec3], point: Vec3) -> (usize, f32, f32) {
    let mut nearest = (0, 0.0, f32::MAX);
    for (index, segment) in points.windows(2).enumerate() {
        let delta = segment[1] - segment[0];
        let denominator = delta.length_squared();
        let fraction = if denominator > f32::EPSILON {
            ((point - segment[0]).dot(delta) / denominator).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let closest = segment[0] + delta * fraction;
        let distance_squared = point.distance_squared(closest);
        if distance_squared < nearest.2 {
            nearest = (index, fraction, distance_squared);
        }
    }
    nearest
}

#[inline]
fn lerp_scalar(start: f32, end: f32, fraction: f32) -> f32 {
    (end - start).mul_add(fraction, start)
}

#[inline]
fn deterministic_fraction(state: &mut u32) -> f32 {
    let mut value = *state;
    value ^= value << 13;
    value ^= value >> 17;
    value ^= value << 5;
    *state = value;
    f32_from_u32(value) / f32_from_u32(u32::MAX)
}

#[inline]
fn signed_nonzero(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

fn orthogonal(direction: Vec3) -> Vec3 {
    let direction = direction.normalize_or_zero();
    if direction == Vec3::ZERO {
        return Vec3::Y;
    }
    let absolute = direction.abs();
    let basis = if absolute.x <= absolute.y && absolute.x <= absolute.z {
        Vec3::X
    } else if absolute.y <= absolute.z {
        Vec3::Y
    } else {
        Vec3::Z
    };
    direction.cross(basis).normalize_or_zero()
}

fn wrap_angle(angle: f32) -> f32 {
    (angle + core::f32::consts::PI).rem_euclid(core::f32::consts::TAU) - core::f32::consts::PI
}
