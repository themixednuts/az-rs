use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    CollisionClass, PhysicalEntityTypes, PhysicsBackend, PhysicsBodyHandle, PhysicsError,
    PhysicsPose, PhysicsScene, PhysicsWorld, RigidBodyBuoyancy, SurfaceIndex,
};

/// Shape and pivot settings for a Cry living entity.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LivingDimensions {
    pub use_capsule: bool,
    pub collider_radius: f32,
    pub collider_half_height: f32,
    pub height_collider: f32,
    pub height_pivot: f32,
    pub height_eye: f32,
    pub height_head: f32,
    pub head_radius: f32,
    pub unprojection_direction: Vec3,
    pub max_unprojection: f32,
    pub ground_contact_epsilon: f32,
}

impl LivingDimensions {
    /// Validates living collider dimensions and unprojection values.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] when a value is non-finite or outside the
    /// accepted range.
    pub fn validate(self) -> Result<(), PhysicsError> {
        for (value, field, positive) in [
            (self.collider_radius, "collider_radius", true),
            (self.collider_half_height, "collider_half_height", true),
            (self.max_unprojection, "max_unprojection", false),
            (self.ground_contact_epsilon, "ground_contact_epsilon", false),
            (self.head_radius, "head_radius", false),
        ] {
            let valid = value.is_finite() && if positive { value > 0.0 } else { value >= 0.0 };
            if !valid {
                return Err(PhysicsError::InvalidLivingConfiguration { field });
            }
        }
        for (value, field) in [
            (self.height_collider, "height_collider"),
            (self.height_pivot, "height_pivot"),
            (self.height_eye, "height_eye"),
            (self.height_head, "height_head"),
        ] {
            if !value.is_finite() {
                return Err(PhysicsError::InvalidLivingConfiguration { field });
            }
        }
        if !self.unprojection_direction.is_finite() {
            return Err(PhysicsError::InvalidLivingConfiguration {
                field: "unprojection_direction",
            });
        }
        Ok(())
    }
}

impl Default for LivingDimensions {
    fn default() -> Self {
        Self {
            use_capsule: false,
            collider_radius: 0.4,
            collider_half_height: 0.6,
            height_collider: 1.1,
            height_pivot: 0.0,
            height_eye: 1.7,
            height_head: 1.9,
            head_radius: 0.0,
            unprojection_direction: Vec3::Z,
            max_unprojection: 0.0,
            ground_contact_epsilon: 0.006,
        }
    }
}

/// Motion and collision settings for a Cry living entity.
#[allow(
    clippy::struct_excessive_bools,
    reason = "these booleans are the directly reflected Cry living configuration surface"
)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LivingDynamics {
    pub mass: f32,
    pub inertia: f32,
    pub inertia_acceleration: f32,
    pub time_impulse_recover: f32,
    pub air_control: f32,
    pub air_resistance: f32,
    pub use_custom_gravity: bool,
    pub gravity: Vec3,
    pub nod_speed: f32,
    pub is_active: bool,
    pub release_ground_collider_when_not_active: bool,
    pub is_swimming: bool,
    pub surface_index: SurfaceIndex,
    pub min_fall_angle: f32,
    pub min_slide_angle: f32,
    pub max_climb_angle: f32,
    pub max_jump_angle: f32,
    pub max_ground_velocity: f32,
    pub collide_with_terrain: bool,
    pub collide_with_static: bool,
    pub collide_with_rigid: bool,
    pub collide_with_sleeping_rigid: bool,
    pub collide_with_living: bool,
    pub collide_with_independent: bool,
    pub record_collisions: bool,
    pub max_recorded_collisions: i32,
}

impl LivingDynamics {
    /// Validates movement, gravity, and slope settings.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] when a scalar is non-finite, a mass is not
    /// positive, or a slope angle is outside zero through 180 degrees.
    pub fn validate(self) -> Result<(), PhysicsError> {
        for (value, field, positive) in [
            (self.mass, "mass", true),
            (self.inertia, "inertia", false),
            (self.inertia_acceleration, "inertia_acceleration", false),
            (self.time_impulse_recover, "time_impulse_recover", false),
            (self.air_control, "air_control", false),
            (self.air_resistance, "air_resistance", false),
            (self.nod_speed, "nod_speed", false),
            (self.max_ground_velocity, "max_ground_velocity", false),
        ] {
            let valid = value.is_finite() && if positive { value > 0.0 } else { value >= 0.0 };
            if !valid {
                return Err(PhysicsError::InvalidLivingConfiguration { field });
            }
        }
        if !self.gravity.is_finite() {
            return Err(PhysicsError::InvalidLivingConfiguration { field: "gravity" });
        }
        if self.air_control > 1.0 {
            return Err(PhysicsError::InvalidLivingConfiguration {
                field: "air_control",
            });
        }
        if self.max_recorded_collisions < 0 {
            return Err(PhysicsError::InvalidLivingConfiguration {
                field: "max_recorded_collisions",
            });
        }
        for (value, field) in [
            (self.min_fall_angle, "min_fall_angle"),
            (self.min_slide_angle, "min_slide_angle"),
            (self.max_climb_angle, "max_climb_angle"),
            (self.max_jump_angle, "max_jump_angle"),
        ] {
            if !value.is_finite() || !(0.0..=90.0).contains(&value) {
                return Err(PhysicsError::InvalidLivingConfiguration { field });
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn collision_types(self) -> PhysicalEntityTypes {
        let mut result = PhysicalEntityTypes::NONE;
        if self.collide_with_terrain {
            result = result.union(PhysicalEntityTypes::TERRAIN);
        }
        if self.collide_with_static {
            result = result.union(PhysicalEntityTypes::STATIC);
        }
        if self.collide_with_rigid || self.collide_with_sleeping_rigid {
            result = result.union(PhysicalEntityTypes::DYNAMIC);
        }
        if self.collide_with_living {
            result = result.union(PhysicalEntityTypes::LIVING);
        }
        if self.collide_with_independent {
            result = result.union(PhysicalEntityTypes::INDEPENDENT);
        }
        result
    }
}

impl Default for LivingDynamics {
    fn default() -> Self {
        Self {
            mass: 80.0,
            inertia: 0.0,
            inertia_acceleration: 0.0,
            time_impulse_recover: 5.0,
            air_control: 0.1,
            air_resistance: 0.0,
            use_custom_gravity: false,
            gravity: Vec3::new(0.0, 0.0, -9.8),
            nod_speed: 60.0,
            is_active: true,
            release_ground_collider_when_not_active: true,
            is_swimming: false,
            surface_index: SurfaceIndex(0),
            min_fall_angle: 70.2,
            min_slide_angle: 36.0,
            max_climb_angle: 54.0,
            max_jump_angle: 54.0,
            max_ground_velocity: 10.0,
            collide_with_terrain: true,
            collide_with_static: true,
            collide_with_rigid: true,
            collide_with_sleeping_rigid: true,
            collide_with_living: true,
            collide_with_independent: true,
            record_collisions: false,
            max_recorded_collisions: 1,
        }
    }
}

/// Complete living-body construction settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LivingBodyConfiguration {
    pub dimensions: LivingDimensions,
    pub dynamics: LivingDynamics,
    pub collision_class: CollisionClass,
    pub friction: f32,
    pub restitution: f32,
    pub surface_pierceability: u8,
}

impl LivingBodyConfiguration {
    /// Validates all living-body and material settings.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] when dimensions, dynamics, or material
    /// coefficients are invalid.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        self.dimensions.validate()?;
        self.dynamics.validate()?;
        for (value, field) in [
            (self.friction, "friction"),
            (self.restitution, "restitution"),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PhysicsError::InvalidLivingConfiguration { field });
            }
        }
        Ok(())
    }
}

impl Default for LivingBodyConfiguration {
    fn default() -> Self {
        Self {
            dimensions: LivingDimensions::default(),
            dynamics: LivingDynamics::default(),
            collision_class: CollisionClass::default(),
            friction: 0.5,
            restitution: 0.0,
            surface_pierceability: 0,
        }
    }
}

/// `pe_action_move::iJump` behavior.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum LivingMoveMode {
    #[default]
    RequestedVelocity = 0,
    SetVelocity = 1,
    AddVelocity = 2,
    ForceFlight = 3,
}

/// Living movement request. `time_step` limits one subsequent simulation step
/// when it is positive, matching `pe_action_move::dt`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LivingMoveAction {
    pub velocity: Vec3,
    pub mode: LivingMoveMode,
    pub time_step: f32,
}

/// Linear impulse request.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ImpulseAction {
    pub impulse: Vec3,
    pub point: Option<Vec3>,
    pub explosion: bool,
    pub apply_during_step: bool,
}

/// Authoritative living-body pose/velocity correction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct SyncLivingAction {
    pub pose: PhysicsPose,
    pub velocity: Vec3,
    pub requested_velocity: Vec3,
}

/// Typed replacement for the Cry `pe_action_*` family.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum PhysicsAction {
    Move(LivingMoveAction),
    Impulse(ImpulseAction),
    AngularImpulse(Vec3),
    Force(Vec3),
    Torque(Vec3),
    Reset,
    Wake(bool),
    SetPose(PhysicsPose),
    SetVelocity(Vec3),
    SetAngularVelocity(Vec3),
    SetMass(f32),
    /// Cry `pe_simulation_params::density`; updates every rigid part mass.
    SetDensity(f32),
    SetLinearDamping(f32),
    SetAngularDamping(f32),
    /// Cry `pe_simulation_params::minEnergy`, not a velocity threshold.
    SetSleepMinEnergy(f32),
    SetBuoyancy(RigidBodyBuoyancy),
    SetSimulated(bool),
    SetLivingDimensions(LivingDimensions),
    SetLivingDynamics(LivingDynamics),
    SyncLiving(SyncLivingAction),
}

/// Result of Cry's `pe_status_check_stance` query.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LivingStanceCheck {
    pub allowed: bool,
    pub unprojection: Vec3,
}

/// Living-specific state corresponding to `pe_status_living`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct LivingStatus {
    pub flying: bool,
    pub time_flying: f32,
    pub camera_offset: Vec3,
    pub velocity: Vec3,
    pub unconstrained_velocity: Vec3,
    pub requested_velocity: Vec3,
    pub ground_velocity: Vec3,
    pub ground_height: f32,
    pub ground_slope: Vec3,
    pub ground_surface: Option<SurfaceIndex>,
    pub ground_body: Option<crate::PhysicsBodyHandle>,
    pub time_since_stance_change: f32,
    pub stuck: bool,
    pub squashed: bool,
}

/// Solver-neutral Cry living-entity command surface.
pub trait LivingBodyCommands {
    /// Dispatches one typed action at a living body.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::ActionRequiresLivingBody`] when the handle names a body
    /// that is not a living entity. The [`PhysicsWorld`] implementation also
    /// returns [`PhysicsError::SceneNotFound`] when the handle's scene is not
    /// open.
    fn apply_living(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError>;

    /// Requests a movement velocity for the next living-entity step.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`LivingBodyCommands::apply_living`] for
    /// [`PhysicsAction::Move`].
    fn request_velocity(
        &mut self,
        body: PhysicsBodyHandle,
        velocity: impl Into<Vec3>,
        mode: LivingMoveMode,
    ) -> Result<(), PhysicsError> {
        self.apply_living(
            body,
            PhysicsAction::Move(LivingMoveAction {
                velocity: velocity.into(),
                mode,
                time_step: 0.0,
            }),
        )
    }

    /// Replaces the living body's collider and pivot dimensions.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`LivingBodyCommands::apply_living`] for
    /// [`PhysicsAction::SetLivingDimensions`], which additionally reports
    /// [`PhysicsError::InvalidLivingConfiguration`] when the backend rejects
    /// `dimensions`.
    fn set_living_dimensions(
        &mut self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<(), PhysicsError> {
        self.apply_living(body, PhysicsAction::SetLivingDimensions(dimensions))
    }

    /// Replaces the living body's motion and collision settings.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`LivingBodyCommands::apply_living`] for
    /// [`PhysicsAction::SetLivingDynamics`], which additionally reports
    /// [`PhysicsError::InvalidLivingConfiguration`] when the backend rejects
    /// `dynamics`.
    fn set_living_dynamics(
        &mut self,
        body: PhysicsBodyHandle,
        dynamics: LivingDynamics,
    ) -> Result<(), PhysicsError> {
        self.apply_living(body, PhysicsAction::SetLivingDynamics(dynamics))
    }
}

impl<B: PhysicsBackend> LivingBodyCommands for PhysicsScene<B> {
    fn apply_living(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.apply_action(body, action)
    }
}

impl LivingBodyCommands for PhysicsWorld {
    fn apply_living(
        &mut self,
        body: PhysicsBodyHandle,
        action: PhysicsAction,
    ) -> Result<(), PhysicsError> {
        self.apply_action(body, action)
    }
}

/// Solver-neutral Cry living-entity query surface.
pub trait LivingBodyQueries {
    /// Reads the living-entity state produced by the last step.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::BodyNotFound`] when `body` is not registered and
    /// [`PhysicsError::ActionRequiresLivingBody`] when it is not a living
    /// entity. The [`PhysicsWorld`] implementation also returns
    /// [`PhysicsError::SceneNotFound`] for a scene that is not open.
    fn living_body_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError>;

    /// Tests whether the body fits the requested stance dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidLivingConfiguration`] from
    /// [`LivingDimensions::validate`] for rejected `dimensions`, and the same
    /// lookup errors as [`LivingBodyQueries::living_body_status`] for `body`.
    fn check_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError>;
}

impl<B: PhysicsBackend> LivingBodyQueries for PhysicsScene<B> {
    fn living_body_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError> {
        self.living_status(body)
    }

    fn check_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError> {
        dimensions.validate()?;
        self.backend().check_living_stance(body, dimensions)
    }
}

impl LivingBodyQueries for PhysicsWorld {
    fn living_body_status(&self, body: PhysicsBodyHandle) -> Result<LivingStatus, PhysicsError> {
        self.living_status(body)
    }

    fn check_stance(
        &self,
        body: PhysicsBodyHandle,
        dimensions: LivingDimensions,
    ) -> Result<LivingStanceCheck, PhysicsError> {
        self.check_living_stance(body, dimensions)
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "tests lock exact reflected default values"
)]
mod tests {
    use super::*;

    #[test]
    fn living_defaults_match_reflected_cry_configuration() {
        let config = LivingBodyConfiguration::default();
        assert!(!config.dimensions.use_capsule);
        assert_eq!(config.dimensions.collider_radius, 0.4);
        assert_eq!(config.dimensions.collider_half_height, 0.6);
        assert_eq!(config.dimensions.height_collider, 1.1);
        assert_eq!(config.dimensions.ground_contact_epsilon, 0.006);
        assert_eq!(config.dynamics.mass, 80.0);
        assert_eq!(config.dynamics.time_impulse_recover, 5.0);
        assert_eq!(config.dynamics.air_control, 0.1);
        assert_eq!(config.dynamics.min_fall_angle, 70.2);
        assert_eq!(config.dynamics.max_climb_angle, 54.0);
        assert_eq!(config.dynamics.max_ground_velocity, 10.0);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn living_collision_flags_form_typed_query_mask() {
        let dynamics = LivingDynamics {
            collide_with_living: false,
            ..LivingDynamics::default()
        };
        assert!(
            !dynamics
                .collision_types()
                .intersects(PhysicalEntityTypes::LIVING)
        );
        assert!(
            dynamics
                .collision_types()
                .intersects(PhysicalEntityTypes::TERRAIN)
        );
    }
}
