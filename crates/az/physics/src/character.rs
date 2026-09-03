use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use az_derive::AzTypeInfo;

use crate::{
    LivingMoveAction, LivingMoveMode, PhysicsBackend, PhysicsBodyHandle, PhysicsEntityId,
    PhysicsError, PhysicsScene, PhysicsWorld,
};

/// Native `Physics::CharacterSupportLevel` discriminants.
///
/// The numeric order is part of the reflected `RockNRoll` contract: supported is
/// zero, unsupported is one, and sliding is two.
#[repr(i32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, Reflect)]
#[serde(rename_all = "snake_case")]
pub enum CharacterSupportState {
    #[default]
    Supported = 0,
    Unsupported = 1,
    Sliding = 2,
}

/// Backend-neutral form of `RockNRoll`'s reflected `CharacterSupportInfo`.
///
/// The native object stores an opaque 24-byte physics identity at the tail.
/// The engine contract exposes the stable body and entity identities instead
/// of preserving that solver-specific ABI representation.
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
#[az_type_info(name = "CharacterSupportInfo", "824E1E63-89EB-49FD-8D85-64AA899A61CA")]
pub struct CharacterSupportInfo {
    pub state: CharacterSupportState,
    pub normal: Vec3,
    pub velocity: Vec3,
    pub distance: f32,
    pub body: Option<PhysicsBodyHandle>,
    pub entity_id: Option<PhysicsEntityId>,
}

impl CharacterSupportInfo {
    #[inline]
    #[must_use]
    pub const fn support_state(self) -> CharacterSupportState {
        self.state
    }

    /// Matches the native `CharacterSupportInfo::IsOnGround`: sliding is not
    /// considered grounded.
    #[inline]
    #[must_use]
    pub const fn is_on_ground(self) -> bool {
        matches!(self.state, CharacterSupportState::Supported)
    }
}

impl Default for CharacterSupportInfo {
    fn default() -> Self {
        Self {
            state: CharacterSupportState::Unsupported,
            normal: Vec3::ZERO,
            velocity: Vec3::ZERO,
            distance: -1.0,
            body: None,
            entity_id: None,
        }
    }
}

/// `RockNRoll` character-controller request surface implemented by every
/// backend-neutral physics scene.
///
/// This is an extension trait so gameplay can depend on the capability without
/// coupling itself to a concrete solver or to a method-heavy scene facade.
pub trait CharacterControllerCommands {
    /// Updates the velocity consumed by the next controller integration.
    ///
    /// # Errors
    ///
    /// Both implementations dispatch [`crate::PhysicsAction::Move`], so they
    /// return [`PhysicsError::BodyNotFound`] for an unregistered handle and
    /// [`PhysicsError::ActionRequiresLivingBody`] when the body is not a
    /// controller. The [`PhysicsWorld`] implementation also returns
    /// [`PhysicsError::SceneNotFound`] when the handle's scene is not open.
    fn request_velocity(
        &mut self,
        body: PhysicsBodyHandle,
        velocity: impl Into<Vec3>,
    ) -> Result<(), PhysicsError>;

    /// Returns the support aggregation produced by the last integration.
    ///
    /// # Errors
    ///
    /// Forwards the backend's `character_support` error, which is
    /// [`PhysicsError::BodyNotFound`] for an unregistered handle and
    /// [`PhysicsError::OperationRequiresCharacterBody`] for a body that is not a
    /// character controller. The [`PhysicsWorld`] implementation also returns
    /// [`PhysicsError::SceneNotFound`] when the handle's scene is not open.
    fn check_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError>;

    /// Explicitly integrates a synchronous (`asynchronous = false`) controller.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidTimeStep`] when `time_step` is non-finite
    /// or not greater than zero, then forwards the backend's
    /// `integrate_character` error: [`PhysicsError::BodyNotFound`] for an
    /// unregistered handle and [`PhysicsError::OperationRequiresCharacterBody`]
    /// for a body that is not a character controller. The [`PhysicsWorld`]
    /// implementation also returns [`PhysicsError::SceneNotFound`] when the
    /// handle's scene is not open.
    fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError>;
}

impl<B: PhysicsBackend> CharacterControllerCommands for PhysicsScene<B> {
    fn request_velocity(
        &mut self,
        body: PhysicsBodyHandle,
        velocity: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply_action(
            body,
            crate::PhysicsAction::Move(LivingMoveAction {
                velocity: velocity.into(),
                mode: LivingMoveMode::RequestedVelocity,
                time_step: 0.0,
            }),
        )
    }

    fn check_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError> {
        self.backend_mut().character_support(body)
    }

    fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        if !time_step.is_finite() || time_step <= 0.0 {
            return Err(PhysicsError::InvalidTimeStep(time_step));
        }
        self.backend_mut().integrate_character(body, time_step)
    }
}

impl CharacterControllerCommands for PhysicsWorld {
    fn request_velocity(
        &mut self,
        body: PhysicsBodyHandle,
        velocity: impl Into<Vec3>,
    ) -> Result<(), PhysicsError> {
        self.apply_action(
            body,
            crate::PhysicsAction::Move(LivingMoveAction {
                velocity: velocity.into(),
                mode: LivingMoveMode::RequestedVelocity,
                time_step: 0.0,
            }),
        )
    }

    fn check_support(
        &mut self,
        body: PhysicsBodyHandle,
    ) -> Result<CharacterSupportInfo, PhysicsError> {
        self.character_support(body)
    }

    fn integrate_character(
        &mut self,
        body: PhysicsBodyHandle,
        time_step: f32,
    ) -> Result<(), PhysicsError> {
        Self::integrate_character(self, body, time_step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_support_discriminants_and_ground_rule_are_stable() {
        assert_eq!(CharacterSupportState::Supported as i32, 0);
        assert_eq!(CharacterSupportState::Unsupported as i32, 1);
        assert_eq!(CharacterSupportState::Sliding as i32, 2);
        assert!(
            CharacterSupportInfo {
                state: CharacterSupportState::Supported,
                ..Default::default()
            }
            .is_on_ground()
        );
        assert!(
            !CharacterSupportInfo {
                state: CharacterSupportState::Sliding,
                ..Default::default()
            }
            .is_on_ground()
        );
    }
}
