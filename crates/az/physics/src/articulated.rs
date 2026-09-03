use bevy_reflect::Reflect;
use glam::{EulerRot, Vec3};
use serde::{Deserialize, Serialize};

use crate::{
    BodyDescriptor, BodyKind, ConstraintAxis, ConstraintAxisConfiguration, ConstraintDescriptor,
    ConstraintDrive, ConstraintSolverModel, ConstraintStatus, ConstraintTarget, PhysicsAction,
    PhysicsBackend, PhysicsBodyHandle, PhysicsConstraintHandle, PhysicsError, PhysicsPose,
    PhysicsScene, PhysicsSceneId,
};

/// Cry articulated-body integration formulation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum ArticulatedSimulationMode {
    #[default]
    JointBased,
    BodyBased,
}

/// Overrides selected when a Cry articulation enters lying mode.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulatedLyingMode {
    pub minimum_contacts: u32,
    pub gravity: Vec3,
    pub damping: f32,
    pub minimum_energy: f32,
    pub simulation_mode: ArticulatedSimulationMode,
}

/// Optional attachment of articulation roots to the world or a host body.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulationGrounding {
    pub target: ConstraintTarget,
    pub target_frame: PhysicsPose,
    pub root_frame: PhysicsPose,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub linear_acceleration: Vec3,
    pub angular_acceleration: Vec3,
    pub inherit_velocity: bool,
}

/// Entity-wide Cry `pe_params_articulated_body` state.
#[expect(
    clippy::struct_excessive_bools,
    reason = "check_collisions, collision_response, self_collisions, apply_external_joint_velocity, and awake are the distinct pe_params_articulated_body flags and must stay one field each"
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulatedBodyConfiguration {
    pub grounding: Option<ArticulationGrounding>,
    pub check_collisions: bool,
    pub collision_response: bool,
    pub self_collisions: bool,
    pub bounce_response_scale: f32,
    pub apply_external_joint_velocity: bool,
    pub awake: bool,
    pub simulation_mode: ArticulatedSimulationMode,
    pub lying_mode: Option<ArticulatedLyingMode>,
}

impl Default for ArticulatedBodyConfiguration {
    fn default() -> Self {
        Self {
            grounding: None,
            check_collisions: true,
            collision_response: true,
            self_collisions: false,
            bounce_response_scale: 1.0,
            apply_external_joint_velocity: false,
            awake: true,
            simulation_mode: ArticulatedSimulationMode::JointBased,
            lying_mode: None,
        }
    }
}

/// Cry `pe_params_joint` state associated with one non-root link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulatedJointConfiguration {
    pub parent_frame: PhysicsPose,
    pub child_frame: PhysicsPose,
    pub axes: crate::ConstraintAxes,
    pub rotation: Vec3,
    pub external_rotation: Vec3,
    pub target_rotation: Vec3,
    pub animation_time_step: Option<f32>,
    pub no_gravity: bool,
    pub self_collision: bool,
    /// Stable part IDs that may self-collide, preserving Cry's bounded list.
    pub self_colliding_parts: Vec<u32>,
}

impl Default for ArticulatedJointConfiguration {
    fn default() -> Self {
        Self {
            parent_frame: PhysicsPose::IDENTITY,
            child_frame: PhysicsPose::IDENTITY,
            axes: crate::ConstraintAxes::SPHERICAL,
            rotation: Vec3::ZERO,
            external_rotation: Vec3::ZERO,
            target_rotation: Vec3::ZERO,
            animation_time_step: None,
            no_gravity: false,
            self_collision: false,
            self_colliding_parts: Vec::new(),
        }
    }
}

/// One link in an authored articulation. Parents must precede their children.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulationLinkDescriptor {
    pub body: BodyDescriptor,
    pub parent: Option<usize>,
    pub joint: Option<ArticulatedJointConfiguration>,
}

/// Complete authored articulation tree/forest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulationDescriptor {
    pub scene: PhysicsSceneId,
    pub configuration: ArticulatedBodyConfiguration,
    pub links: Vec<ArticulationLinkDescriptor>,
}

impl ArticulationDescriptor {
    /// Validates topology and all nested body/joint values.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] for an empty/non-topological tree, a non-
    /// articulated link body, invalid grounding, or invalid nested values.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if self.links.is_empty() {
            return Err(PhysicsError::InvalidArticulationConfiguration { field: "links" });
        }
        if !self.configuration.bounce_response_scale.is_finite()
            || self.configuration.bounce_response_scale < 0.0
        {
            return Err(PhysicsError::InvalidArticulationConfiguration {
                field: "bounce response scale",
            });
        }
        if let Some(grounding) = self.configuration.grounding {
            validate_vector(grounding.linear_velocity, "ground linear velocity")?;
            validate_vector(grounding.angular_velocity, "ground angular velocity")?;
            validate_vector(grounding.linear_acceleration, "ground linear acceleration")?;
            validate_vector(
                grounding.angular_acceleration,
                "ground angular acceleration",
            )?;
            if let ConstraintTarget::Body(host) = grounding.target
                && host.scene() != self.scene
            {
                return Err(PhysicsError::ConstraintSceneMismatch {
                    parent: host.scene(),
                    child: self.scene,
                });
            }
        }
        if let Some(lying) = self.configuration.lying_mode
            && (lying.minimum_contacts == 0
                || !lying.gravity.is_finite()
                || !lying.damping.is_finite()
                || lying.damping < 0.0
                || !lying.minimum_energy.is_finite()
                || lying.minimum_energy < 0.0)
        {
            return Err(PhysicsError::InvalidArticulationConfiguration {
                field: "lying mode",
            });
        }

        let mut roots = 0;
        for (index, link) in self.links.iter().enumerate() {
            link.body.validate()?;
            if !matches!(link.body.kind, BodyKind::Articulated(_)) {
                return Err(PhysicsError::InvalidArticulationConfiguration {
                    field: "link body kind",
                });
            }
            match (link.parent, &link.joint) {
                (None, None) => roots += 1,
                (Some(parent), Some(joint)) if parent < index => validate_joint(joint)?,
                (Some(_), Some(_)) => {
                    return Err(PhysicsError::InvalidArticulationConfiguration {
                        field: "parent order",
                    });
                }
                _ => {
                    return Err(PhysicsError::InvalidArticulationConfiguration {
                        field: "root/joint pairing",
                    });
                }
            }
        }
        if roots == 0 {
            return Err(PhysicsError::InvalidArticulationConfiguration { field: "roots" });
        }
        Ok(())
    }
}

fn validate_joint(joint: &ArticulatedJointConfiguration) -> Result<(), PhysicsError> {
    for (field, value) in [
        ("joint rotation", joint.rotation),
        ("joint external rotation", joint.external_rotation),
        ("joint target rotation", joint.target_rotation),
    ] {
        validate_vector(value, field)?;
    }
    if joint
        .animation_time_step
        .is_some_and(|step| !step.is_finite() || step <= 0.0)
    {
        return Err(PhysicsError::InvalidArticulationConfiguration {
            field: "joint animation time step",
        });
    }
    ConstraintDescriptor {
        parent: ConstraintTarget::World,
        child: PhysicsBodyHandle::in_scene(PhysicsSceneId::DEFAULT, std::num::NonZeroU64::MIN),
        parent_frame: joint.parent_frame,
        child_frame: joint.child_frame,
        axes: joint.axes,
        linear_coupling: None,
        angular_coupling: None,
        solver_model: ConstraintSolverModel::ReducedCoordinate,
        enabled: true,
        contacts_enabled: joint.self_collision,
        break_force: None,
        break_torque: None,
        break_impulse: None,
        damping: 0.0,
        sensor_radius: 0.0,
    }
    .validate()
}

fn validate_vector(value: Vec3, field: &'static str) -> Result<(), PhysicsError> {
    if !value.is_finite() {
        return Err(PhysicsError::InvalidArticulationConfiguration { field });
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct RuntimeJoint {
    handle: PhysicsConstraintHandle,
    descriptor: ConstraintDescriptor,
    external_rotation: Vec3,
}

/// Runtime ownership of all bodies and constraints in one articulation.
#[derive(bevy_ecs::component::Component, Debug, Clone)]
pub struct Articulation {
    scene: PhysicsSceneId,
    configuration: ArticulatedBodyConfiguration,
    bodies: Vec<PhysicsBodyHandle>,
    parents: Vec<Option<usize>>,
    joints: Vec<Option<RuntimeJoint>>,
}

impl Articulation {
    #[must_use]
    pub const fn scene(&self) -> PhysicsSceneId {
        self.scene
    }

    #[must_use]
    pub fn bodies(&self) -> &[PhysicsBodyHandle] {
        &self.bodies
    }

    pub fn roots(&self) -> impl Iterator<Item = PhysicsBodyHandle> + '_ {
        self.bodies
            .iter()
            .zip(&self.parents)
            .filter_map(|(body, parent)| parent.is_none().then_some(*body))
    }
}

impl AsRef<[PhysicsBodyHandle]> for Articulation {
    fn as_ref(&self) -> &[PhysicsBodyHandle] {
        self.bodies()
    }
}

/// Animation update for a single articulated joint.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulatedJointTarget {
    pub rotation: Vec3,
    pub external_rotation: Vec3,
    pub stiffness: Vec3,
    pub damping: Vec3,
    pub maximum_torque: Vec3,
    pub animation_time_step: f32,
}

/// Reconstructed Cry `pe_status_joint` state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ArticulatedJointStatus {
    pub body: PhysicsBodyHandle,
    pub parent: Option<PhysicsBodyHandle>,
    pub rotation: Vec3,
    pub external_rotation: Vec3,
    pub angular_velocity: Vec3,
    pub constraint: Option<ConstraintStatus>,
}

pub fn create_articulation<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    descriptor: &ArticulationDescriptor,
) -> Result<Articulation, PhysicsError> {
    descriptor.validate()?;
    let mut bodies = Vec::with_capacity(descriptor.links.len());
    let mut joints: Vec<Option<RuntimeJoint>> = Vec::with_capacity(descriptor.links.len());

    if let Err(error) = create_link_bodies(scene, descriptor, &mut bodies) {
        rollback(scene, &mut bodies, &mut joints);
        return Err(error);
    }
    if let Err(error) = create_link_joints(scene, descriptor, &bodies, &mut joints) {
        rollback(scene, &mut bodies, &mut joints);
        return Err(error);
    }
    if let Err(error) = seed_grounding_velocity(scene, descriptor, &bodies) {
        rollback(scene, &mut bodies, &mut joints);
        return Err(error);
    }

    Ok(Articulation {
        scene: descriptor.scene,
        configuration: descriptor.configuration.clone(),
        bodies,
        parents: descriptor.links.iter().map(|link| link.parent).collect(),
        joints,
    })
}

/// Creates one backend body per link, applying the entity-wide sleep, gravity,
/// and collision overrides. The caller is responsible for rollback on error.
fn create_link_bodies<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    descriptor: &ArticulationDescriptor,
    bodies: &mut Vec<PhysicsBodyHandle>,
) -> Result<(), PhysicsError> {
    for link in &descriptor.links {
        let mut body = link.body.clone();
        if let BodyKind::Articulated(configuration) = &mut body.kind {
            configuration.start_asleep = !descriptor.configuration.awake;
            if link.joint.as_ref().is_some_and(|joint| joint.no_gravity) {
                configuration.gravity_enabled = false;
            }
        }
        if !descriptor.configuration.check_collisions {
            for collider in &mut body.colliders {
                collider.simulated = false;
            }
        } else if !descriptor.configuration.collision_response {
            for collider in &mut body.colliders {
                collider.sensor = true;
                collider.simulated = true;
            }
        }
        match scene.create_body(&body) {
            Ok(handle) if handle.scene() == descriptor.scene => bodies.push(handle),
            Ok(handle) => {
                let _ = scene.remove_body(handle);
                return Err(PhysicsError::BackendInvariant(
                    "articulation link was created in a different scene",
                ));
            }
            Err(error) => {
                return Err(error);
            }
        }
    }
    Ok(())
}

/// Creates the joint constraint for every child link and the grounding
/// constraint for every root. The caller is responsible for rollback on error.
fn create_link_joints<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    descriptor: &ArticulationDescriptor,
    bodies: &[PhysicsBodyHandle],
    joints: &mut Vec<Option<RuntimeJoint>>,
) -> Result<(), PhysicsError> {
    for (index, link) in descriptor.links.iter().enumerate() {
        let constraint = if let Some(parent) = link.parent {
            let Some(joint) = link.joint.as_ref() else {
                return Err(PhysicsError::BackendInvariant(
                    "validated articulation child lost its joint",
                ));
            };
            Some(joint_descriptor(
                bodies[parent].into(),
                bodies[index],
                joint,
                &descriptor.configuration,
            ))
        } else if let Some(grounding) = descriptor.configuration.grounding {
            let mut constraint = ConstraintDescriptor::fixed(grounding.target, bodies[index]);
            constraint.parent_frame = grounding.target_frame;
            constraint.child_frame = grounding.root_frame;
            constraint.solver_model = solver_model(descriptor.configuration.simulation_mode);
            constraint.contacts_enabled = descriptor.configuration.self_collisions;
            Some(constraint)
        } else {
            None
        };

        let runtime = if let Some(constraint) = constraint {
            match scene.create_constraint(&constraint) {
                Ok(handle) => Some(RuntimeJoint {
                    handle,
                    descriptor: constraint,
                    external_rotation: link
                        .joint
                        .as_ref()
                        .map_or(Vec3::ZERO, |joint| joint.external_rotation),
                }),
                Err(error) => {
                    return Err(error);
                }
            }
        } else {
            None
        };
        joints.push(runtime);
    }
    Ok(())
}

/// Seeds every root link with the grounding target's velocity when the
/// articulation asks to inherit it.
fn seed_grounding_velocity<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    descriptor: &ArticulationDescriptor,
    bodies: &[PhysicsBodyHandle],
) -> Result<(), PhysicsError> {
    if let Some(grounding) = descriptor.configuration.grounding
        && grounding.inherit_velocity
    {
        let (linear, angular) = match grounding.target {
            ConstraintTarget::World => (grounding.linear_velocity, grounding.angular_velocity),
            ConstraintTarget::Body(host) => {
                let status = scene.body_status(host)?;
                (
                    status.linear_velocity + grounding.linear_velocity,
                    status.angular_velocity + grounding.angular_velocity,
                )
            }
        };
        for (body, link) in bodies.iter().zip(&descriptor.links) {
            if link.parent.is_none() {
                scene.apply_action(*body, PhysicsAction::SetVelocity(linear))?;
                scene.apply_action(*body, PhysicsAction::SetAngularVelocity(angular))?;
            }
        }
    }
    Ok(())
}

const fn solver_model(mode: ArticulatedSimulationMode) -> ConstraintSolverModel {
    match mode {
        ArticulatedSimulationMode::JointBased => ConstraintSolverModel::ReducedCoordinate,
        ArticulatedSimulationMode::BodyBased => ConstraintSolverModel::Impulse,
    }
}

fn joint_descriptor(
    parent: ConstraintTarget,
    child: PhysicsBodyHandle,
    joint: &ArticulatedJointConfiguration,
    articulation: &ArticulatedBodyConfiguration,
) -> ConstraintDescriptor {
    let mut axes = joint.axes;
    let target = joint.target_rotation + joint.external_rotation;
    for (axis, target) in [
        (ConstraintAxis::AngularX, target.x),
        (ConstraintAxis::AngularY, target.y),
        (ConstraintAxis::AngularZ, target.z),
    ] {
        let mut configuration = axes.get(axis);
        if let Some(drive) = &mut configuration.drive {
            drive.target_position = target;
            axes.set(axis, configuration);
        }
    }
    ConstraintDescriptor {
        parent,
        child,
        parent_frame: joint.parent_frame,
        child_frame: joint.child_frame,
        axes,
        linear_coupling: None,
        angular_coupling: None,
        solver_model: solver_model(articulation.simulation_mode),
        enabled: true,
        contacts_enabled: articulation.self_collisions || joint.self_collision,
        break_force: None,
        break_torque: None,
        break_impulse: None,
        damping: 0.0,
        sensor_radius: 0.0,
    }
}

pub fn set_joint_target<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    articulation: &mut Articulation,
    link: usize,
    target: ArticulatedJointTarget,
) -> Result<(), PhysicsError> {
    if !target.rotation.is_finite()
        || !target.external_rotation.is_finite()
        || !target.stiffness.is_finite()
        || !target.damping.is_finite()
        || !target.maximum_torque.is_finite()
        || !target.animation_time_step.is_finite()
        || target.animation_time_step <= 0.0
        || target.stiffness.min_element() < 0.0
        || target.damping.min_element() < 0.0
        || target.maximum_torque.min_element() < 0.0
    {
        return Err(PhysicsError::InvalidArticulationConfiguration {
            field: "joint target",
        });
    }
    let joint = articulation
        .joints
        .get_mut(link)
        .and_then(Option::as_mut)
        .ok_or(PhysicsError::ArticulationLinkNotFound(link))?;
    let velocity = if articulation.configuration.apply_external_joint_velocity {
        (target.external_rotation - joint.external_rotation) / target.animation_time_step
    } else {
        Vec3::ZERO
    };
    let position = target.rotation + target.external_rotation;
    for (axis, position, velocity, stiffness, damping, maximum_force) in [
        (
            ConstraintAxis::AngularX,
            position.x,
            velocity.x,
            target.stiffness.x,
            target.damping.x,
            target.maximum_torque.x,
        ),
        (
            ConstraintAxis::AngularY,
            position.y,
            velocity.y,
            target.stiffness.y,
            target.damping.y,
            target.maximum_torque.y,
        ),
        (
            ConstraintAxis::AngularZ,
            position.z,
            velocity.z,
            target.stiffness.z,
            target.damping.z,
            target.maximum_torque.z,
        ),
    ] {
        joint.descriptor.axes.set(
            axis,
            ConstraintAxisConfiguration {
                motion: joint.descriptor.axes.get(axis).motion,
                drive: Some(ConstraintDrive {
                    target_position: position,
                    target_velocity: velocity,
                    stiffness,
                    damping,
                    maximum_force,
                }),
            },
        );
    }
    scene.update_constraint(joint.handle, &joint.descriptor)?;
    joint.external_rotation = target.external_rotation;
    Ok(())
}

pub fn joint_statuses<B: PhysicsBackend>(
    scene: &PhysicsScene<B>,
    articulation: &Articulation,
) -> Result<Vec<ArticulatedJointStatus>, PhysicsError> {
    let mut statuses = Vec::with_capacity(articulation.bodies.len());
    for (index, body) in articulation.bodies.iter().copied().enumerate() {
        let body_status = scene.body_status(body)?;
        let (parent, parent_pose, parent_angular_velocity) =
            if let Some(parent_index) = articulation.parents[index] {
                let parent = articulation.bodies[parent_index];
                let status = scene.body_status(parent)?;
                (Some(parent), status.pose, status.angular_velocity)
            } else {
                (None, PhysicsPose::IDENTITY, Vec3::ZERO)
            };
        let (rotation, constraint, external_rotation) =
            if let Some(joint) = &articulation.joints[index] {
                let relative = (parent_pose * joint.descriptor.parent_frame).inverse()
                    * (body_status.pose * joint.descriptor.child_frame);
                let (x, y, z) = relative.rotation.to_euler(EulerRot::XYZ);
                (
                    Vec3::new(x, y, z),
                    Some(scene.constraint_status(joint.handle)?),
                    joint.external_rotation,
                )
            } else {
                (Vec3::ZERO, None, Vec3::ZERO)
            };
        statuses.push(ArticulatedJointStatus {
            body,
            parent,
            rotation,
            external_rotation,
            angular_velocity: body_status.angular_velocity - parent_angular_velocity,
            constraint,
        });
    }
    Ok(statuses)
}

pub fn remove_articulation<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    articulation: &mut Articulation,
) -> Result<(), PhysicsError> {
    let mut first_error = None;
    for joint in articulation
        .joints
        .iter_mut()
        .rev()
        .filter_map(std::option::Option::take)
    {
        if let Err(error) = scene.remove_constraint(joint.handle)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    for body in articulation.bodies.drain(..).rev() {
        if let Err(error) = scene.remove_body(body)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    articulation.parents.clear();
    first_error.map_or(Ok(()), Err)
}

fn rollback<B: PhysicsBackend>(
    scene: &mut PhysicsScene<B>,
    bodies: &mut Vec<PhysicsBodyHandle>,
    joints: &mut [Option<RuntimeJoint>],
) {
    for joint in joints
        .iter_mut()
        .rev()
        .filter_map(std::option::Option::take)
    {
        let _ = scene.remove_constraint(joint.handle);
    }
    for body in bodies.drain(..).rev() {
        let _ = scene.remove_body(body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColliderConfiguration, RigidBodyConfiguration};

    fn link(parent: Option<usize>) -> ArticulationLinkDescriptor {
        ArticulationLinkDescriptor {
            body: BodyDescriptor {
                entity_id: None,
                pose: PhysicsPose::IDENTITY,
                kind: BodyKind::Articulated(RigidBodyConfiguration::default()),
                colliders: vec![ColliderConfiguration::default()],
            },
            parent,
            joint: parent.map(|_| ArticulatedJointConfiguration::default()),
        }
    }

    #[test]
    fn topology_requires_parents_to_precede_children() {
        let descriptor = ArticulationDescriptor {
            scene: PhysicsSceneId::DEFAULT,
            configuration: ArticulatedBodyConfiguration::default(),
            links: vec![link(Some(1)), link(None)],
        };
        assert_eq!(
            descriptor.validate(),
            Err(PhysicsError::InvalidArticulationConfiguration {
                field: "parent order",
            })
        );
    }
}
