use bevy_reflect::Reflect;
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::{
    ColliderShape, ColliderTag, CollisionClass, CollisionFilter, PhysicalEntityTypes,
    PhysicsBodyHandle, PhysicsEntityId, PhysicsError, PhysicsPose, SurfaceIndex,
};

/// Shared filtering applied to ray, overlap, and shape-cast queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub struct SpatialQueryFilter {
    pub ignore_entity_ids: Vec<PhysicsEntityId>,
    pub ignore_bodies: Vec<PhysicsBodyHandle>,
    pub physical_entity_types: PhysicalEntityTypes,
    pub include_sensors: bool,
    /// Cry collision class used to filter the query against each candidate's
    /// `SCollisionClass`. `None` accepts every collision class.
    pub collision_class: Option<CollisionClass>,
    /// Compiled `RockNRoll` category filter. Candidates without a
    /// compiled category filter do not match a filtered query.
    pub collision_filter: Option<CollisionFilter>,
}

impl Default for SpatialQueryFilter {
    fn default() -> Self {
        Self {
            ignore_entity_ids: Vec::new(),
            ignore_bodies: Vec::new(),
            physical_entity_types: PhysicalEntityTypes::ALL,
            include_sensors: true,
            collision_class: None,
            collision_filter: None,
        }
    }
}

/// Reflected `LmbrCentral` ray-cast request shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RayCastConfiguration {
    pub origin: Vec3,
    pub direction: Vec3,
    pub max_distance: f32,
    pub ignore_entity_ids: Vec<PhysicsEntityId>,
    pub ignore_bodies: Vec<PhysicsBodyHandle>,
    pub max_hits: usize,
    pub pierces_surfaces_greater_than: i32,
    pub physical_entity_types: PhysicalEntityTypes,
    pub include_sensors: bool,
    pub collision_class: Option<CollisionClass>,
    pub collision_filter: Option<CollisionFilter>,
}

impl RayCastConfiguration {
    #[must_use]
    pub fn filter(&self) -> SpatialQueryFilter {
        SpatialQueryFilter {
            ignore_entity_ids: self.ignore_entity_ids.clone(),
            ignore_bodies: self.ignore_bodies.clone(),
            physical_entity_types: self.physical_entity_types,
            include_sensors: self.include_sensors,
            collision_class: self.collision_class,
            collision_filter: self.collision_filter,
        }
    }
}

impl RayCastConfiguration {
    pub const MAX_SURFACE_PIERCEABILITY: i32 = 15;

    /// Validates the normalized direction, range, and requested hit count.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError`] for an invalid direction, distance, or zero
    /// maximum hit count.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        if !self.direction.is_finite() || (self.direction.length_squared() - 1.0).abs() > 1.0e-4 {
            return Err(PhysicsError::InvalidRayDirection);
        }
        if !self.max_distance.is_finite() || self.max_distance < 0.0 {
            return Err(PhysicsError::InvalidRayDistance(self.max_distance));
        }
        if self.max_hits == 0 {
            return Err(PhysicsError::InvalidRayHitCount);
        }
        Ok(())
    }
}

impl Default for RayCastConfiguration {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            direction: Vec3::Y,
            max_distance: 100.0,
            ignore_entity_ids: Vec::new(),
            ignore_bodies: Vec::new(),
            max_hits: 1,
            pierces_surfaces_greater_than: Self::MAX_SURFACE_PIERCEABILITY,
            physical_entity_types: PhysicalEntityTypes::ALL,
            include_sensors: true,
            collision_class: None,
            collision_filter: None,
        }
    }
}

/// One world-space ray intersection.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RayCastHit {
    pub distance: f32,
    pub position: Vec3,
    pub normal: Vec3,
    pub entity_id: Option<PhysicsEntityId>,
    pub body: PhysicsBodyHandle,
    pub surface_index: SurfaceIndex,
    pub surface_pierceability: u8,
    pub collider_tag: ColliderTag,
}

/// World-space axis-aligned overlap bounds used by the legacy `LmbrCentral`
/// gather request bus.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct AabbQuery {
    pub min: Vec3,
    pub max: Vec3,
    pub physical_entity_types: PhysicalEntityTypes,
    pub max_results: usize,
}

impl AabbQuery {
    /// Rejects degenerate bounds and empty result budgets.
    ///
    /// # Errors
    ///
    /// Returns [`PhysicsError::InvalidSpatialQueryBounds`] when either corner is
    /// non-finite or `max` is not strictly greater than `min` on every axis, and
    /// [`PhysicsError::InvalidSpatialQueryResultCount`] when `max_results` is
    /// zero.
    pub fn validate(self) -> Result<(), PhysicsError> {
        if !self.min.is_finite() || !self.max.is_finite() || !self.max.cmpgt(self.min).all() {
            return Err(PhysicsError::InvalidSpatialQueryBounds);
        }
        if self.max_results == 0 {
            return Err(PhysicsError::InvalidSpatialQueryResultCount);
        }
        Ok(())
    }
}

/// Arbitrary-shape overlap request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ShapeOverlapConfiguration {
    pub shape: ColliderShape,
    pub pose: PhysicsPose,
    pub filter: SpatialQueryFilter,
    pub max_results: usize,
}

impl ShapeOverlapConfiguration {
    /// Rejects unusable shapes and empty result budgets.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`ColliderShape::validate`], then returns
    /// [`PhysicsError::InvalidSpatialQueryResultCount`] when `max_results` is
    /// zero.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        self.shape.validate()?;
        if self.max_results == 0 {
            return Err(PhysicsError::InvalidSpatialQueryResultCount);
        }
        Ok(())
    }
}

/// Sweeps one shape along a normalized direction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ShapeCastConfiguration {
    pub shape: ColliderShape,
    pub pose: PhysicsPose,
    pub direction: Vec3,
    pub max_distance: f32,
    pub target_distance: f32,
    pub stop_at_penetration: bool,
    pub filter: SpatialQueryFilter,
    pub max_results: usize,
}

impl ShapeCastConfiguration {
    /// Rejects unusable shapes, sweep directions, and distances.
    ///
    /// # Errors
    ///
    /// Forwards the error from [`ColliderShape::validate`], then returns
    /// [`PhysicsError::InvalidRayDirection`] for a non-finite or non-unit
    /// `direction`, [`PhysicsError::InvalidRayDistance`] for a non-finite or
    /// negative `max_distance`, [`PhysicsError::InvalidColliderScalar`] with
    /// field `target_distance` for a non-finite or negative `target_distance`,
    /// and [`PhysicsError::InvalidSpatialQueryResultCount`] when `max_results`
    /// is zero.
    pub fn validate(&self) -> Result<(), PhysicsError> {
        self.shape.validate()?;
        if !self.direction.is_finite() || (self.direction.length_squared() - 1.0).abs() > 1.0e-4 {
            return Err(PhysicsError::InvalidRayDirection);
        }
        if !self.max_distance.is_finite() || self.max_distance < 0.0 {
            return Err(PhysicsError::InvalidRayDistance(self.max_distance));
        }
        if !self.target_distance.is_finite() || self.target_distance < 0.0 {
            return Err(PhysicsError::InvalidColliderScalar {
                field: "target_distance",
            });
        }
        if self.max_results == 0 {
            return Err(PhysicsError::InvalidSpatialQueryResultCount);
        }
        Ok(())
    }
}

/// Identity and material returned by overlap queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub struct OverlapHit {
    pub body: PhysicsBodyHandle,
    pub entity_id: Option<PhysicsEntityId>,
    pub surface_index: SurfaceIndex,
    pub collider_tag: ColliderTag,
}

/// First impact returned by a shape cast.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Reflect)]
pub struct ShapeCastHit {
    pub distance: f32,
    pub position: Vec3,
    pub normal: Vec3,
    pub body: PhysicsBodyHandle,
    pub entity_id: Option<PhysicsEntityId>,
    pub surface_index: SurfaceIndex,
    pub surface_pierceability: u8,
    pub collider_tag: ColliderTag,
}

/// Ordered piercing hits followed by at most one blocking hit.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Reflect)]
pub struct RayCastResult {
    piercing_hits: Vec<RayCastHit>,
    blocking_hit: Option<RayCastHit>,
}

impl RayCastResult {
    #[must_use]
    pub fn hit_count(&self) -> usize {
        self.piercing_hits.len() + usize::from(self.blocking_hit.is_some())
    }

    #[must_use]
    pub const fn has_blocking_hit(&self) -> bool {
        self.blocking_hit.is_some()
    }

    #[must_use]
    pub const fn blocking_hit(&self) -> Option<&RayCastHit> {
        self.blocking_hit.as_ref()
    }

    #[must_use]
    pub fn piercing_hits(&self) -> &[RayCastHit] {
        &self.piercing_hits
    }

    /// Iterates piercing hits in distance order followed by the optional
    /// blocking hit without allocating an aggregate result vector.
    pub fn iter(&self) -> impl Iterator<Item = &RayCastHit> {
        self.piercing_hits.iter().chain(self.blocking_hit.iter())
    }

    pub fn add_piercing_hit(&mut self, hit: RayCastHit) {
        self.piercing_hits.push(hit);
    }

    pub const fn set_blocking_hit(&mut self, hit: RayCastHit) {
        self.blocking_hit = Some(hit);
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
    fn defaults_match_lmbrcentral_reflection() {
        let query = RayCastConfiguration::default();
        assert_eq!(query.origin, Vec3::ZERO);
        assert_eq!(query.direction, Vec3::Y);
        assert_eq!(query.max_distance, 100.0);
        assert_eq!(query.max_hits, 1);
        assert_eq!(query.pierces_surfaces_greater_than, 15);
        assert_eq!(query.physical_entity_types, PhysicalEntityTypes::ALL);
        assert!(query.validate().is_ok());
    }
}
