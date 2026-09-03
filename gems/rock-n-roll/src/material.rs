//! Backend-neutral `RockNRoll` contact-material coefficients.

use std::borrow::Borrow;

use bevy::prelude::Reflect;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The friction and restitution coefficients attached to one collider.
///
/// `RockNRoll` combines each coefficient by multiplying the two collider values.
/// Keeping that policy on the value type prevents a backend adapter from
/// silently substituting its own default combine rule.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
pub struct ContactMaterial {
    friction: f32,
    restitution: f32,
}

impl ContactMaterial {
    /// Builds finite contact-material coefficients.
    ///
    /// Signed values are preserved because the recovered native constraint
    /// path multiplies authored coefficients without an observed sign clamp.
    ///
    /// # Errors
    ///
    /// Returns [`ContactMaterialError`] when either coefficient is non-finite.
    pub const fn try_new(friction: f32, restitution: f32) -> Result<Self, ContactMaterialError> {
        if !friction.is_finite() {
            return Err(ContactMaterialError::NonFiniteCoefficient {
                field: "friction",
                value: friction,
            });
        }
        if !restitution.is_finite() {
            return Err(ContactMaterialError::NonFiniteCoefficient {
                field: "restitution",
                value: restitution,
            });
        }

        Ok(Self {
            friction,
            restitution,
        })
    }

    #[inline]
    #[must_use]
    pub const fn friction(self) -> f32 {
        self.friction
    }

    #[inline]
    #[must_use]
    pub const fn restitution(self) -> f32 {
        self.restitution
    }

    /// Applies `RockNRoll`'s component-wise multiplicative combine policy.
    #[inline]
    #[must_use]
    pub fn combine(&self, other: impl Borrow<Self>) -> Self {
        let other = other.borrow();
        Self {
            friction: self.friction * other.friction,
            restitution: self.restitution * other.restitution,
        }
    }
}

impl Default for ContactMaterial {
    fn default() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.5,
        }
    }
}

impl TryFrom<(f32, f32)> for ContactMaterial {
    type Error = ContactMaterialError;

    fn try_from((friction, restitution): (f32, f32)) -> Result<Self, Self::Error> {
        Self::try_new(friction, restitution)
    }
}

impl From<ContactMaterial> for (f32, f32) {
    fn from(value: ContactMaterial) -> Self {
        (value.friction, value.restitution)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum ContactMaterialError {
    #[error("{field} coefficient must be finite, got {value}")]
    NonFiniteCoefficient { field: &'static str, value: f32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_rigid_body_configuration_defaults() {
        assert_eq!(
            ContactMaterial::default(),
            ContactMaterial::try_new(0.5, 0.5).unwrap()
        );
    }

    #[test]
    fn combine_multiplies_both_coefficients() {
        let first = ContactMaterial::try_new(0.5, 0.25).unwrap();
        let second = ContactMaterial::try_new(0.2, 0.8).unwrap();

        assert_eq!(
            first.combine(second),
            ContactMaterial::try_new(0.1, 0.2).unwrap()
        );
    }

    #[test]
    fn conversion_traits_preserve_the_pair_without_allocation() {
        let material = ContactMaterial::try_from((0.75, 0.125)).unwrap();
        assert_eq!(<(f32, f32)>::from(material), (0.75, 0.125));
    }

    #[test]
    fn non_finite_coefficients_are_rejected() {
        assert!(ContactMaterial::try_new(f32::NAN, 0.0).is_err());
        assert!(ContactMaterial::try_new(0.0, f32::INFINITY).is_err());
    }
}
