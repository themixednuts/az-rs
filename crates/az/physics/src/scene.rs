use std::collections::BTreeMap;

use bevy_math::bounding::Aabb3d;
use glam::Vec3;

use crate::{
    AabbQuery, ArticulatedJointStatus, ArticulatedJointTarget, Articulation,
    ArticulationDescriptor, BodyDescriptor, BodyStatus, CharacterSupportInfo, ConstraintDescriptor,
    ConstraintStatus, DeformableTargetVertices, LinkedSoftBodyStatusRef, LivingDimensions,
    LivingStanceCheck, LivingStatus, OverlapHit, ParticleStatus, PhysicsAction, PhysicsBodyHandle,
    PhysicsConstraintBackend, PhysicsConstraintHandle, PhysicsError, PhysicsInteraction,
    PhysicsLinkedSoftBodyBackend, PhysicsParticleBackend, PhysicsRopeBackend, PhysicsSceneId,
    PhysicsSoftBodyBackend, PhysicsVehicleBackend, RayCastConfiguration, RayCastResult, RopeStatus,
    RopeVolumetricPressure, ShapeCastConfiguration, ShapeCastHit, ShapeOverlapConfiguration,
    SoftBodyAttachmentUpdate, SoftBodyImpulse, SoftBodyPressure, SoftBodyStatus, VehicleAbilities,
    VehicleDriveAction, VehicleStatus, VehicleWheelStatus,
};

/// Solver adapter required by an engine physics scene.
#[allow(
    clippy::missing_errors_doc,
    reason = "each backend reports the shared PhysicsError contract documented on PhysicsScene"
)]
pub trait PhysicsBackend:
    PhysicsConstraintBackend
    + PhysicsLinkedSoftBodyBackend
    + PhysicsParticleBackend
    + PhysicsRopeBackend
    + PhysicsSoftBodyBackend
    + PhysicsVehicleBackend
    + std::fmt::Debug
    + Send
    + Sync
    + 'static
{
    fn gravity(&self) -> Vec3;
    fn set_gravity(&mut self, gravity: Vec3);

    fn create_body(
        &mut self,
        descriptor: &BodyDescriptor,
    ) -> Result<PhysicsBodyHandle, PhysicsError>;
    fn remove_body(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError>;
    fn apply_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError>;
    fn body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError>;
    /// Writes the dynamic contact/constraint island containing `body` into
    /// caller-owned storage. The seed body is always first.
    fn connected_bodies(
        &self,
        body: PhysicsBodyHandle,
        output: &mut Vec<PhysicsBodyHandle>,
    ) -> Result<(), PhysicsError> {
        self.body_status(body)?;
        output.clear();
        output.push(body);
        Ok(())
    }
    fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError>;
    fn living_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError>;
    fn check_living_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError>;
    fn character_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError>;
    fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError>;
    fn ray_cast(&self, query: &RayCastConfiguration) -> Result<RayCastResult, PhysicsError>;
    fn overlap_aabb(&self, query: AabbQuery) -> Result<Vec<OverlapHit>, PhysicsError>;
    fn overlap_shape(
        &self,
        query: &ShapeOverlapConfiguration,
    ) -> Result<Vec<OverlapHit>, PhysicsError>;
    fn cast_shape(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Option<ShapeCastHit>, PhysicsError> {
        Ok(self.cast_shape_all(query)?.into_iter().next())
    }
    fn cast_shape_all(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Vec<ShapeCastHit>, PhysicsError>;
    fn drain_interactions(&mut self) -> Vec<PhysicsInteraction>;
    fn step(&mut self, time_step: f32) -> Result<(), PhysicsError>;
}

impl<B: PhysicsBackend + ?Sized> PhysicsBackend for Box<B> {
    fn gravity(&self) -> Vec3 {
        (**self).gravity()
    }

    fn set_gravity(&mut self, gravity: Vec3) {
        (**self).set_gravity(gravity);
    }

    fn create_body(
        &mut self,
        descriptor: &BodyDescriptor,
    ) -> Result<PhysicsBodyHandle, PhysicsError> {
        (**self).create_body(descriptor)
    }

    fn remove_body(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        (**self).remove_body(body)
    }

    fn apply_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        (**self).apply_action(body, action)
    }

    fn body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError> {
        (**self).body_status(body)
    }

    fn connected_bodies(
        &self,
        body: PhysicsBodyHandle,
        output: &mut Vec<PhysicsBodyHandle>,
    ) -> Result<(), PhysicsError> {
        (**self).connected_bodies(body, output)
    }

    fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError> {
        (**self).body_aabb(body)
    }

    fn living_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError> {
        (**self).living_status(body)
    }

    fn check_living_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError> {
        (**self).check_living_stance(body, dimensions)
    }

    fn character_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError> {
        (**self).character_support(body)
    }

    fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        (**self).integrate_character(body, time_step)
    }

    fn ray_cast(&self, query: &RayCastConfiguration) -> Result<RayCastResult, PhysicsError> {
        (**self).ray_cast(query)
    }

    fn overlap_aabb(&self, query: AabbQuery) -> Result<Vec<OverlapHit>, PhysicsError> {
        (**self).overlap_aabb(query)
    }

    fn overlap_shape(
        &self,
        query: &ShapeOverlapConfiguration,
    ) -> Result<Vec<OverlapHit>, PhysicsError> {
        (**self).overlap_shape(query)
    }

    fn cast_shape(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Option<ShapeCastHit>, PhysicsError> {
        (**self).cast_shape(query)
    }

    fn cast_shape_all(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Vec<ShapeCastHit>, PhysicsError> {
        (**self).cast_shape_all(query)
    }

    fn drain_interactions(&mut self) -> Vec<PhysicsInteraction> {
        (**self).drain_interactions()
    }

    fn step(&mut self, time_step: f32) -> Result<(), PhysicsError> {
        (**self).step(time_step)
    }
}

/// Owns one isolated simulation scene and hides its concrete solver.
#[derive(bevy_ecs::resource::Resource, Debug)]
pub struct PhysicsScene<B> {
    backend: B,
}

/// Creates one concrete solver backend per isolated engine scene.
pub trait PhysicsBackendFactory: std::fmt::Debug + Send + Sync + 'static {
    fn create(&self, scene: PhysicsSceneId) -> Box<dyn PhysicsBackend>;
}

/// Engine-owned registry of isolated physics scenes.
///
/// Body operations route through the scene embedded in
/// [`PhysicsBodyHandle`]. Spatial queries are intentionally performed on an
/// explicitly selected [`PhysicsScene`] so a server query cannot accidentally
/// cross instance boundaries.
#[derive(bevy_ecs::resource::Resource)]
pub struct PhysicsWorld {
    factory: Box<dyn PhysicsBackendFactory>,
    scenes: BTreeMap<PhysicsSceneId, PhysicsScene<Box<dyn PhysicsBackend>>>,
}

impl std::fmt::Debug for PhysicsWorld {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PhysicsWorld")
            .field("factory", &self.factory)
            .field("scenes", &self.scenes.keys().collect::<Vec<_>>())
            .finish()
    }
}

#[allow(
    clippy::missing_errors_doc,
    reason = "scene operations forward the shared PhysicsError variants from validation and the backend"
)]
impl<B: PhysicsBackend> PhysicsScene<B> {
    #[must_use]
    pub const fn new(backend: B) -> Self {
        Self { backend }
    }

    #[must_use]
    pub const fn backend(&self) -> &B {
        &self.backend
    }

    pub const fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    #[must_use]
    pub fn into_backend(self) -> B {
        self.backend
    }

    #[must_use]
    pub fn gravity(&self) -> Vec3 {
        self.backend.gravity()
    }

    pub fn set_gravity(&mut self, gravity: Vec3) {
        self.backend.set_gravity(gravity);
    }

    pub fn create_body(
        &mut self,
        descriptor: &BodyDescriptor,
    ) -> Result<PhysicsBodyHandle, PhysicsError> {
        descriptor.validate()?;
        self.backend.create_body(descriptor)
    }

    pub fn remove_body(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        self.backend.remove_body(body)
    }

    pub fn create_articulation(
        &mut self,
        descriptor: &ArticulationDescriptor,
    ) -> Result<Articulation, PhysicsError> {
        crate::articulated::create_articulation(self, descriptor)
    }

    pub fn remove_articulation(
        &mut self,
        articulation: &mut Articulation,
    ) -> Result<(), PhysicsError> {
        crate::articulated::remove_articulation(self, articulation)
    }

    pub fn set_articulation_joint_target(
        &mut self,
        articulation: &mut Articulation,
        link: usize,
        target: ArticulatedJointTarget,
    ) -> Result<(), PhysicsError> {
        crate::articulated::set_joint_target(self, articulation, link, target)
    }

    pub fn articulation_joint_statuses(
        &self,
        articulation: &Articulation,
    ) -> Result<Vec<ArticulatedJointStatus>, PhysicsError> {
        crate::articulated::joint_statuses(self, articulation)
    }

    pub fn create_constraint(
        &mut self,
        descriptor: &ConstraintDescriptor,
    ) -> Result<PhysicsConstraintHandle, PhysicsError> {
        descriptor.validate()?;
        self.backend.create_constraint(descriptor)
    }

    pub fn update_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
        descriptor: &ConstraintDescriptor,
    ) -> Result<(), PhysicsError> {
        descriptor.validate()?;
        self.backend.update_constraint(constraint, descriptor)
    }

    pub fn remove_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<(), PhysicsError> {
        self.backend.remove_constraint(constraint)
    }

    pub fn constraint_status(
        &self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<ConstraintStatus, PhysicsError> {
        self.backend.constraint_status(constraint)
    }

    pub fn apply_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.backend.apply_action(body, action)
    }

    pub fn body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError> {
        self.backend.body_status(body)
    }

    pub fn particle_status(&self, body: PhysicsBodyHandle) -> Result<ParticleStatus, PhysicsError> {
        self.backend.particle_status(body)
    }

    pub fn take_particle_collision(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<bool, PhysicsError> {
        self.backend.take_particle_collision(body)
    }

    pub fn set_rope_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        action.validate()?;
        self.backend.set_rope_target_vertices(body, action)
    }

    pub fn notify_rope_attachment_moved(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<(), PhysicsError> {
        self.backend.notify_rope_attachment_moved(body)
    }

    pub fn apply_rope_volumetric_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: RopeVolumetricPressure,
    ) -> Result<(), PhysicsError> {
        pressure.validate()?;
        self.backend.apply_rope_volumetric_pressure(body, pressure)
    }

    pub fn write_rope_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut RopeStatus,
    ) -> Result<(), PhysicsError> {
        self.backend.write_rope_status(body, output)
    }

    pub fn update_soft_body_attachments(
        &mut self,
        body: PhysicsBodyHandle,
        update: &SoftBodyAttachmentUpdate,
    ) -> Result<(), PhysicsError> {
        update.validate()?;
        self.backend.update_soft_body_attachments(body, update)
    }

    pub fn set_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        action.validate()?;
        self.backend.set_soft_body_target_vertices(body, action)
    }

    pub fn apply_soft_body_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: SoftBodyImpulse,
    ) -> Result<(), PhysicsError> {
        impulse.validate()?;
        self.backend.apply_soft_body_impulse(body, impulse)
    }

    pub fn apply_soft_body_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: SoftBodyPressure,
    ) -> Result<(), PhysicsError> {
        pressure.validate()?;
        self.backend.apply_soft_body_pressure(body, pressure)
    }

    pub fn write_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut SoftBodyStatus,
    ) -> Result<(), PhysicsError> {
        self.backend.write_soft_body_status(body, output)
    }

    pub fn linked_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
    ) -> Result<LinkedSoftBodyStatusRef<'_>, PhysicsError> {
        self.backend.linked_soft_body_status(body)
    }

    pub fn set_linked_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        target: DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        target.validate()?;
        self.backend
            .set_linked_soft_body_target_vertices(body, target)
    }

    pub fn apply_vehicle_drive(
        &mut self,
        body: PhysicsBodyHandle,
        action: VehicleDriveAction,
    ) -> Result<(), PhysicsError> {
        action.validate()?;
        self.backend.apply_vehicle_drive(body, action)
    }

    pub fn vehicle_status(&self, body: PhysicsBodyHandle) -> Result<VehicleStatus, PhysicsError> {
        self.backend.vehicle_status(body)
    }

    pub fn vehicle_wheel_status(
        &self,
        body: PhysicsBodyHandle,
        wheel: usize,
    ) -> Result<VehicleWheelStatus, PhysicsError> {
        self.backend.vehicle_wheel_status(body, wheel)
    }

    pub fn vehicle_abilities(
        &self,
        body: PhysicsBodyHandle,
        steer: Option<f32>,
    ) -> Result<VehicleAbilities, PhysicsError> {
        if steer.is_some_and(|steer| !steer.is_finite()) {
            return Err(PhysicsError::InvalidVehicleConfiguration {
                field: "abilities steer",
            });
        }
        self.backend.vehicle_abilities(body, steer)
    }

    pub fn connected_bodies(
        &self,
        body: PhysicsBodyHandle,
        output: &mut Vec<PhysicsBodyHandle>,
    ) -> Result<(), PhysicsError> {
        self.backend.connected_bodies(body, output)
    }

    pub fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError> {
        self.backend.body_aabb(body)
    }

    pub fn living_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError> {
        self.backend.living_status(body)
    }

    pub fn check_living_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError> {
        dimensions.validate()?;
        self.backend.check_living_stance(body, dimensions)
    }

    pub fn ray_cast(&self, query: &RayCastConfiguration) -> Result<RayCastResult, PhysicsError> {
        query.validate()?;
        self.backend.ray_cast(query)
    }

    pub fn overlap_aabb(&self, query: AabbQuery) -> Result<Vec<OverlapHit>, PhysicsError> {
        query.validate()?;
        self.backend.overlap_aabb(query)
    }

    pub fn overlap_shape(
        &self,
        query: &ShapeOverlapConfiguration,
    ) -> Result<Vec<OverlapHit>, PhysicsError> {
        query.validate()?;
        self.backend.overlap_shape(query)
    }

    pub fn cast_shape(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Option<ShapeCastHit>, PhysicsError> {
        query.validate()?;
        self.backend.cast_shape(query)
    }

    pub fn cast_shape_all(
        &self,
        query: &ShapeCastConfiguration,
    ) -> Result<Vec<ShapeCastHit>, PhysicsError> {
        query.validate()?;
        self.backend.cast_shape_all(query)
    }

    pub fn drain_interactions(&mut self) -> Vec<PhysicsInteraction> {
        self.backend.drain_interactions()
    }

    pub fn step(&mut self, time_step: f32) -> Result<(), PhysicsError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(PhysicsError::InvalidTimeStep(time_step));
        }
        self.backend.step(time_step)
    }
}

#[allow(
    clippy::missing_errors_doc,
    reason = "world operations forward the selected scene's shared PhysicsError contract"
)]
impl PhysicsWorld {
    #[must_use]
    pub fn new(factory: impl PhysicsBackendFactory) -> Self {
        Self {
            factory: Box::new(factory),
            scenes: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn contains_scene(&self, scene: PhysicsSceneId) -> bool {
        self.scenes.contains_key(&scene)
    }

    pub fn ensure_scene(
        &mut self,
        scene: PhysicsSceneId,
    ) -> &mut PhysicsScene<Box<dyn PhysicsBackend>> {
        self.scenes
            .entry(scene)
            .or_insert_with(|| PhysicsScene::new(self.factory.create(scene)))
    }

    pub fn remove_scene(
        &mut self,
        scene: PhysicsSceneId,
    ) -> Option<PhysicsScene<Box<dyn PhysicsBackend>>> {
        self.scenes.remove(&scene)
    }

    pub fn scene(
        &self,
        scene: PhysicsSceneId,
    ) -> Result<&PhysicsScene<Box<dyn PhysicsBackend>>, PhysicsError> {
        self.scenes
            .get(&scene)
            .ok_or(PhysicsError::SceneNotFound(scene))
    }

    pub fn scene_mut(
        &mut self,
        scene: PhysicsSceneId,
    ) -> Result<&mut PhysicsScene<Box<dyn PhysicsBackend>>, PhysicsError> {
        self.scenes
            .get_mut(&scene)
            .ok_or(PhysicsError::SceneNotFound(scene))
    }

    pub fn create_body(
        &mut self,
        scene: PhysicsSceneId,
        descriptor: &BodyDescriptor,
    ) -> Result<PhysicsBodyHandle, PhysicsError> {
        let body = self.ensure_scene(scene).create_body(descriptor)?;
        if body.scene() != scene {
            return Err(PhysicsError::BackendInvariant(
                "backend returned a body handle for a different physics scene",
            ));
        }
        Ok(body)
    }

    pub fn remove_body(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?.remove_body(body)
    }

    pub fn create_articulation(
        &mut self,
        descriptor: &ArticulationDescriptor,
    ) -> Result<Articulation, PhysicsError> {
        let scene = descriptor.scene;
        let articulation = self.ensure_scene(scene).create_articulation(descriptor)?;
        if articulation.scene() != scene {
            return Err(PhysicsError::BackendInvariant(
                "backend returned an articulation for a different physics scene",
            ));
        }
        Ok(articulation)
    }

    pub fn remove_articulation(
        &mut self,
        articulation: &mut Articulation,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(articulation.scene())?
            .remove_articulation(articulation)
    }

    pub fn set_articulation_joint_target(
        &mut self,
        articulation: &mut Articulation,
        link: usize,
        target: ArticulatedJointTarget,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(articulation.scene())?
            .set_articulation_joint_target(articulation, link, target)
    }

    pub fn articulation_joint_statuses(
        &self,
        articulation: &Articulation,
    ) -> Result<Vec<ArticulatedJointStatus>, PhysicsError> {
        self.scene(articulation.scene())?
            .articulation_joint_statuses(articulation)
    }

    pub fn create_constraint(
        &mut self,
        descriptor: &ConstraintDescriptor,
    ) -> Result<PhysicsConstraintHandle, PhysicsError> {
        let scene = descriptor.scene();
        let constraint = self.ensure_scene(scene).create_constraint(descriptor)?;
        if constraint.scene() != scene {
            return Err(PhysicsError::BackendInvariant(
                "backend returned a constraint handle for a different physics scene",
            ));
        }
        Ok(constraint)
    }

    pub fn update_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
        descriptor: &ConstraintDescriptor,
    ) -> Result<(), PhysicsError> {
        if constraint.scene() != descriptor.scene() {
            return Err(PhysicsError::ConstraintSceneMismatch {
                parent: constraint.scene(),
                child: descriptor.scene(),
            });
        }
        self.scene_mut(constraint.scene())?
            .update_constraint(constraint, descriptor)
    }

    pub fn remove_constraint(
        &mut self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(constraint.scene())?
            .remove_constraint(constraint)
    }

    pub fn constraint_status(
        &self,
        constraint: PhysicsConstraintHandle,
    ) -> Result<ConstraintStatus, PhysicsError> {
        self.scene(constraint.scene())?
            .constraint_status(constraint)
    }

    pub fn apply_action(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?.apply_action(body, action)
    }

    pub fn body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError> {
        self.scene(body.scene())?.body_status(body)
    }

    pub fn particle_status(&self, body: PhysicsBodyHandle) -> Result<ParticleStatus, PhysicsError> {
        self.scene(body.scene())?.particle_status(body)
    }

    pub fn take_particle_collision(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<bool, PhysicsError> {
        self.scene_mut(body.scene())?.take_particle_collision(body)
    }

    pub fn set_rope_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .set_rope_target_vertices(body, action)
    }

    pub fn notify_rope_attachment_moved(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .notify_rope_attachment_moved(body)
    }

    pub fn apply_rope_volumetric_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: RopeVolumetricPressure,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .apply_rope_volumetric_pressure(body, pressure)
    }

    pub fn write_rope_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut RopeStatus,
    ) -> Result<(), PhysicsError> {
        self.scene(body.scene())?.write_rope_status(body, output)
    }

    pub fn update_soft_body_attachments(
        &mut self,
        body: PhysicsBodyHandle,
        update: &SoftBodyAttachmentUpdate,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .update_soft_body_attachments(body, update)
    }

    pub fn set_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        action: &DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .set_soft_body_target_vertices(body, action)
    }

    pub fn apply_soft_body_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: SoftBodyImpulse,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .apply_soft_body_impulse(body, impulse)
    }

    pub fn apply_soft_body_pressure(
        &mut self,
        body: PhysicsBodyHandle,
        pressure: SoftBodyPressure,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .apply_soft_body_pressure(body, pressure)
    }

    pub fn write_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
        output: &mut SoftBodyStatus,
    ) -> Result<(), PhysicsError> {
        self.scene(body.scene())?
            .write_soft_body_status(body, output)
    }

    pub fn linked_soft_body_status(
        &self,
        body: PhysicsBodyHandle,
    ) -> Result<LinkedSoftBodyStatusRef<'_>, PhysicsError> {
        self.scene(body.scene())?.linked_soft_body_status(body)
    }

    pub fn set_linked_soft_body_target_vertices(
        &mut self,
        body: PhysicsBodyHandle,
        target: DeformableTargetVertices,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .set_linked_soft_body_target_vertices(body, target)
    }

    pub fn apply_vehicle_drive(
        &mut self,
        body: PhysicsBodyHandle,
        action: VehicleDriveAction,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(body.scene())?
            .apply_vehicle_drive(body, action)
    }

    pub fn vehicle_status(&self, body: PhysicsBodyHandle) -> Result<VehicleStatus, PhysicsError> {
        self.scene(body.scene())?.vehicle_status(body)
    }

    pub fn vehicle_wheel_status(
        &self,
        body: PhysicsBodyHandle,
        wheel: usize,
    ) -> Result<VehicleWheelStatus, PhysicsError> {
        self.scene(body.scene())?.vehicle_wheel_status(body, wheel)
    }

    pub fn vehicle_abilities(
        &self,
        body: PhysicsBodyHandle,
        steer: Option<f32>,
    ) -> Result<VehicleAbilities, PhysicsError> {
        self.scene(body.scene())?.vehicle_abilities(body, steer)
    }

    pub fn connected_bodies(
        &self,
        body: PhysicsBodyHandle,
        output: &mut Vec<PhysicsBodyHandle>,
    ) -> Result<(), PhysicsError> {
        self.scene(body.scene())?.connected_bodies(body, output)
    }

    pub fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError> {
        self.scene(body.scene())?.body_aabb(body)
    }

    pub fn living_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError> {
        self.scene(body.scene())?.living_status(body)
    }

    pub fn check_living_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError> {
        self.scene(body.scene())?
            .check_living_stance(body, dimensions)
    }

    pub fn character_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError> {
        self.scene_mut(body.scene())?
            .backend_mut()
            .character_support(body)
    }

    pub fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(PhysicsError::InvalidTimeStep(time_step));
        }
        self.scene_mut(body.scene())?
            .backend_mut()
            .integrate_character(body, time_step)
    }

    pub fn set_gravity(
        &mut self,
        scene: PhysicsSceneId,
        gravity: Vec3,
    ) -> Result<(), PhysicsError> {
        self.scene_mut(scene)?.set_gravity(gravity);
        Ok(())
    }

    pub fn set_gravity_all(&mut self, gravity: Vec3) {
        for scene in self.scenes.values_mut() {
            scene.set_gravity(gravity);
        }
    }

    pub fn step_all(&mut self, time_step: f32) -> Result<(), PhysicsError> {
        for scene in self.scenes.values_mut() {
            scene.step(time_step)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn drain_interactions(&mut self) -> Vec<PhysicsInteraction> {
        self.scenes
            .values_mut()
            .flat_map(PhysicsScene::drain_interactions)
            .collect()
    }
}
