use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{PhysicsError, PhysicsPose};

/// Owned, solver-neutral form of Cry `pe_action_target_vtx`.
///
/// Ropes use `points` directly (or capture their current vertices when it is
/// `None`). Soft bodies additionally use `host`: when absent, Cry resolves the
/// first attached vertex's host frame before capturing the current pose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct DeformableTargetVertices {
    pub points: Option<Vec<Vec3>>,
    pub host: Option<PhysicsPose>,
}

impl DeformableTargetVertices {
    /// Validates the common `pe_action_target_vtx` payload. Entity-specific
    /// vertex-count validation remains with the receiving body.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidDeformableTarget`] for non-finite data or
    /// a target with fewer than two vertices.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if self
            .points
            .as_ref()
            .is_some_and(|points| points.len() < 2 || points.iter().any(|point| !point.is_finite()))
            || self.host.is_some_and(|host| {
                let rotation_length = host.rotation.length_squared();
                !host.translation.is_finite()
                    || !host.rotation.is_finite()
                    || !rotation_length.is_finite()
                    || rotation_length <= f32::EPSILON
            })
        {
            return Err(PhysicsError::InvalidDeformableTarget);
        }
        Ok(())
    }
}
