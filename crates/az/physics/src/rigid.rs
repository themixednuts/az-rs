use bevy_math::bounding::Aabb3d;
use glam::Vec3;

use crate::{
    BodyStatus, ImpulseAction, PhysicsAction, PhysicsBackend, PhysicsBodyHandle, PhysicsError,
    PhysicsPose, PhysicsScene, PhysicsWorld, RigidBodyBuoyancy,
};

/// Solver-neutral rigid-body command surface.
///
/// This is an extension trait on a physics scene rather than a second request
/// object or solver wrapper. Callers remain generic over the backend and the
/// typed [`PhysicsAction`] family remains the single dispatch path.
pub trait RigidBodyCommands {
    /// Dispatches one typed action at `body`.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered in
    /// the addressed scene, plus whichever validation variant the backend
    /// raises for an action it rejects. The [`PhysicsWorld`] implementation also
    /// returns [`PhysicsError::SceneNotFound`] when the handle names a scene
    /// that is not open.
    fn apply(&mut self, body: PhysicsBodyHandle, action: PhysicsAction)
    -> Result<(), PhysicsError>;

    /// Applies a linear impulse at the body's center of mass.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::Impulse`].
    fn add_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply(
            body,
            PhysicsAction::Impulse(ImpulseAction {
                impulse: impulse.into(),
                point: None,
                explosion: false,
                apply_during_step: false,
            }),
        )
    }

    /// Applies a linear impulse at a world-space point on the body.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::Impulse`].
    fn add_impulse_at_point(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: impl Into<Vec3>,
        point: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply(
            body,
            PhysicsAction::Impulse(ImpulseAction {
                impulse: impulse.into(),
                point: Some(point.into()),
                explosion: false,
                apply_during_step: false,
            }),
        )
    }

    /// Applies an angular impulse about the body's center of mass.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::AngularImpulse`].
    fn add_angular_impulse(
        &mut self,
        body: PhysicsBodyHandle,
        impulse: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::AngularImpulse(impulse.into()))
    }

    /// Preserves Lumberyard's deprecated request-bus operation exactly.
    ///
    /// `PhysicsComponentBus.h` retains this member for source compatibility,
    /// but its forwarding implementation is deliberately a no-op.  Keeping it
    /// here avoids a second compatibility facade while making the lack of a
    /// solver action explicit.
    ///
    /// # Errors
    ///
    /// Never fails: the default body dispatches no action and always returns
    /// `Ok(())`, matching the no-op Lumberyard shipped.
    #[deprecated(note = "Lumberyard retained this operation as a no-op")]
    fn add_angular_impulse_at_point(
        &mut self,
        _body: PhysicsBodyHandle,
        _impulse: impl Into<Vec3>,
        _point: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        Ok(())
    }

    /// Teleports the body to a world-space pose.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetPose`].
    fn set_pose(
        &mut self,
        body: PhysicsBodyHandle,
        pose: impl Into<PhysicsPose>,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetPose(pose.into()))
    }

    /// Overwrites the body's linear velocity.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetVelocity`].
    fn set_velocity(
        &mut self,
        body: PhysicsBodyHandle,
        velocity: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetVelocity(velocity.into()))
    }

    /// Overwrites the body's angular velocity.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetAngularVelocity`].
    fn set_angular_velocity(
        &mut self,
        body: PhysicsBodyHandle,
        velocity: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetAngularVelocity(velocity.into()))
    }

    /// Overwrites the body's total mass.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetMass`], which additionally reports
    /// [`PhysicsError::InvalidRigidBodyScalar`] when the backend rejects `mass`.
    fn set_mass(&mut self, body: PhysicsBodyHandle, mass: f32) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetMass(mass))
    }

    /// Recomputes every rigid part's mass from a uniform density.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetDensity`], which additionally reports
    /// [`PhysicsError::InvalidRigidBodyScalar`] when the backend rejects
    /// `density`.
    fn set_density(&mut self, body: PhysicsBodyHandle, density: f32) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetDensity(density))
    }

    /// Overwrites the body's linear damping coefficient.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetLinearDamping`], which additionally reports
    /// [`PhysicsError::InvalidRigidBodyScalar`] when the backend rejects
    /// `damping`.
    fn set_linear_damping(
        &mut self,
        body: PhysicsBodyHandle,
        damping: f32,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetLinearDamping(damping))
    }

    /// Overwrites the body's angular damping coefficient.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetAngularDamping`], which additionally reports
    /// [`PhysicsError::InvalidRigidBodyScalar`] when the backend rejects
    /// `damping`.
    fn set_angular_damping(
        &mut self,
        body: PhysicsBodyHandle,
        damping: f32,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetAngularDamping(damping))
    }

    /// Overwrites the sleep energy threshold used by the solver.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetSleepMinEnergy`], which additionally reports
    /// [`PhysicsError::InvalidRigidBodyScalar`] when the backend rejects
    /// `minimum_energy`.
    fn set_sleep_min_energy(
        &mut self,
        body: PhysicsBodyHandle,
        minimum_energy: f32,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetSleepMinEnergy(minimum_energy))
    }

    /// Overwrites the body's buoyancy multipliers.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetBuoyancy`], which additionally reports
    /// [`PhysicsError::InvalidFluidConfiguration`] when the backend rejects the
    /// multipliers.
    fn set_buoyancy(
        &mut self,
        body: PhysicsBodyHandle,
        configuration: impl Into<RigidBodyBuoyancy>,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetBuoyancy(configuration.into()))
    }

    /// Enables or disables solver integration for the body.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::SetSimulated`].
    fn set_simulated(
        &mut self,
        body: PhysicsBodyHandle,
        simulated: bool,
    ) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::SetSimulated(simulated))
    }

    /// Wakes the body from solver sleep.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::Wake`].
    fn force_awake(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::Wake(true))
    }

    /// Puts the body to sleep immediately.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyCommands::apply`] for
    /// [`PhysicsAction::Wake`].
    fn force_asleep(&mut self, body: PhysicsBodyHandle) -> Result<(), PhysicsError> {
        self.apply(body, PhysicsAction::Wake(false))
    }
}

impl<B: PhysicsBackend> RigidBodyCommands for PhysicsScene<B> {
    #[inline]
    fn apply(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.apply_action(body, action)
    }
}

impl RigidBodyCommands for PhysicsWorld {
    #[inline]
    fn apply(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.apply_action(body, action)
    }
}

/// Solver-neutral rigid-body status surface.
pub trait RigidBodyQueries {
    /// Reads the body state produced by the last solver step.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered in
    /// the addressed scene. The [`PhysicsWorld`] implementation also returns
    /// [`PhysicsError::SceneNotFound`] when the handle names a scene that is not
    /// open.
    fn rigid_body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError>;

    /// Reads the body's linear velocity.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn velocity(&self, body: PhysicsBodyHandle) -> Result<Vec3, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.linear_velocity)
    }

    /// Reads the body's angular velocity.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn angular_velocity(&self, body: PhysicsBodyHandle) -> Result<Vec3, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.angular_velocity)
    }

    /// Reads the body's linear acceleration.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn acceleration(&self, body: PhysicsBodyHandle) -> Result<Vec3, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.linear_acceleration)
    }

    /// Reads the body's angular acceleration.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn angular_acceleration(&self, body: PhysicsBodyHandle) -> Result<Vec3, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.angular_acceleration)
    }

    /// Reads the body's total mass.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn mass(&self, body: PhysicsBodyHandle) -> Result<f32, PhysicsError> {
        self.rigid_body_status(body).map(|status| status.mass)
    }

    /// Reads the body's uniform density.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn density(&self, body: PhysicsBodyHandle) -> Result<f32, PhysicsError> {
        self.rigid_body_status(body).map(|status| status.density)
    }

    /// Reads the body's linear damping coefficient.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn linear_damping(&self, body: PhysicsBodyHandle) -> Result<f32, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.linear_damping)
    }

    /// Reads the body's angular damping coefficient.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn angular_damping(&self, body: PhysicsBodyHandle) -> Result<f32, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.angular_damping)
    }

    /// Reads the body's sleep energy threshold.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn sleep_min_energy(&self, body: PhysicsBodyHandle) -> Result<f32, PhysicsError> {
        self.rigid_body_status(body)
            .map(|status| status.sleep_min_energy)
    }

    /// Reads the body's buoyancy multipliers.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn buoyancy(&self, body: PhysicsBodyHandle) -> Result<RigidBodyBuoyancy, PhysicsError> {
        self.rigid_body_status(body).map(|status| status.buoyancy)
    }

    /// Reports whether the solver currently considers the body awake.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn is_awake(&self, body: PhysicsBodyHandle) -> Result<bool, PhysicsError> {
        self.rigid_body_status(body).map(|status| status.awake)
    }

    /// Reports whether the solver currently integrates the body.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`RigidBodyQueries::rigid_body_status`].
    fn is_simulated(&self, body: PhysicsBodyHandle) -> Result<bool, PhysicsError> {
        self.rigid_body_status(body).map(|status| status.simulated)
    }

    /// Reads the body's world-space bounding box.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered in
    /// the addressed scene. The [`PhysicsWorld`] implementation also returns
    /// [`PhysicsError::SceneNotFound`] when the handle names a scene that is not
    /// open.
    fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError>;
}

impl<B: PhysicsBackend> RigidBodyQueries for PhysicsScene<B> {
    #[inline]
    fn rigid_body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError> {
        self.body_status(body)
    }

    #[inline]
    fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError> {
        Self::body_aabb(self, body)
    }
}

impl RigidBodyQueries for PhysicsWorld {
    #[inline]
    fn rigid_body_status(&self, body: PhysicsBodyHandle) -> Result<BodyStatus, PhysicsError> {
        self.body_status(body)
    }

    #[inline]
    fn body_aabb(&self, body: PhysicsBodyHandle) -> Result<Aabb3d, PhysicsError> {
        Self::body_aabb(self, body)
    }
}
