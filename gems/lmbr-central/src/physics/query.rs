//! Reflected `LmbrCentral` scene-query value types.

use az_core::component::EntityId;
use az_derive::AzTypeInfo;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const RAY_CAST_CONFIGURATION_TYPE_ID: Uuid = uuid!("FC4E13C6-33D7-4015-91C4-ECBE08F7C5BE");
pub const RAY_CAST_HIT_TYPE_ID: Uuid = uuid!("3D8FA68C-A145-44B4-BA18-F3405D83A9DF");
pub const RAY_CAST_RESULT_TYPE_ID: Uuid = uuid!("9A32A294-0BC5-4931-B594-9EAAF95A6B78");

const fn invalid_ray_distance() -> f32 {
    -1.0
}

/// Serialized `RayCastConfiguration` version 1 data.
#[derive(AzTypeInfo, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "RayCastConfiguration", RAY_CAST_CONFIGURATION_TYPE_ID)]
pub struct RayCastConfiguration {
    #[serde(default)]
    pub origin: Vec3,
    #[serde(default)]
    pub direction: Vec3,
    #[serde(rename = "maxDistance", default)]
    pub max_distance: f32,
    #[serde(rename = "ignoreEntityIds", default)]
    pub ignore_entity_ids: Vec<EntityId>,
    #[serde(rename = "maxHits", default)]
    pub max_hits: u64,
    #[serde(rename = "piercesSurfacesGreaterThan", default)]
    pub pierces_surfaces_greater_than: i32,
    #[serde(rename = "physicalEntityTypes", default)]
    pub physical_entity_types: i32,
}

impl Default for RayCastConfiguration {
    fn default() -> Self {
        Self {
            origin: Vec3::ZERO,
            direction: Vec3::Y,
            max_distance: 100.0,
            ignore_entity_ids: Vec::new(),
            max_hits: 1,
            pierces_surfaces_greater_than: 15,
            physical_entity_types: 31,
        }
    }
}

/// Serialized `RayCastHit` version 1 data.
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "RayCastHit", RAY_CAST_HIT_TYPE_ID)]
pub struct RayCastHit {
    #[serde(default = "invalid_ray_distance")]
    pub distance: f32,
    #[serde(default)]
    pub position: Vec3,
    #[serde(default)]
    pub normal: Vec3,
    #[serde(rename = "entityId", default)]
    pub entity_id: EntityId,
}

impl RayCastHit {
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.distance >= 0.0
    }
}

impl Default for RayCastHit {
    fn default() -> Self {
        Self {
            distance: -1.0,
            position: Vec3::ZERO,
            normal: Vec3::ZERO,
            entity_id: EntityId::INVALID,
        }
    }
}

/// Runtime result returned by the legacy ray-cast behavior API.
#[derive(AzTypeInfo, Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "RayCastResult", RAY_CAST_RESULT_TYPE_ID)]
#[reflect(Serialize, Deserialize)]
pub struct RayCastResult {
    #[serde(skip)]
    #[reflect(ignore)]
    blocking_hit: RayCastHit,
    #[serde(skip)]
    #[reflect(ignore)]
    piercing_hits: Vec<RayCastHit>,
}

impl RayCastResult {
    #[must_use]
    pub fn hit_count(&self) -> usize {
        self.piercing_hits.len() + usize::from(self.blocking_hit.is_valid())
    }

    #[must_use]
    pub fn hit(&self, index: usize) -> Option<&RayCastHit> {
        self.piercing_hits.get(index).or_else(|| {
            (index == self.piercing_hits.len() && self.blocking_hit.is_valid())
                .then_some(&self.blocking_hit)
        })
    }

    #[must_use]
    pub const fn has_blocking_hit(&self) -> bool {
        self.blocking_hit.is_valid()
    }

    #[must_use]
    pub fn blocking_hit(&self) -> Option<&RayCastHit> {
        self.has_blocking_hit().then_some(&self.blocking_hit)
    }

    #[must_use]
    pub const fn piercing_hit_count(&self) -> usize {
        self.piercing_hits.len()
    }

    #[must_use]
    pub fn piercing_hit(&self, index: usize) -> Option<&RayCastHit> {
        self.piercing_hits.get(index)
    }

    pub const fn set_blocking_hit(&mut self, hit: RayCastHit) {
        self.blocking_hit = hit;
    }

    pub fn add_piercing_hit(&mut self, hit: RayCastHit) {
        self.piercing_hits.push(hit);
    }
}

impl TryFrom<&RayCastConfiguration> for az_physics::RayCastConfiguration {
    type Error = az_physics::PhysicsError;

    fn try_from(value: &RayCastConfiguration) -> Result<Self, Self::Error> {
        let max_hits = usize::try_from(value.max_hits)
            .map_err(|_| az_physics::PhysicsError::InvalidRayHitCount)?;
        let output = Self {
            origin: value.origin,
            direction: value.direction,
            max_distance: value.max_distance,
            ignore_entity_ids: value
                .ignore_entity_ids
                .iter()
                .copied()
                .filter(|id| id.is_valid())
                .map(|id| az_physics::PhysicsEntityId(id.value()))
                .collect(),
            max_hits,
            pierces_surfaces_greater_than: value.pierces_surfaces_greater_than,
            // The native config stores the mask signed; reinterpret the bits
            // rather than reject a mask with the high bit set.
            physical_entity_types: az_physics::PhysicalEntityTypes::from_bits(
                value.physical_entity_types.cast_unsigned(),
            ),
            ..Self::default()
        };
        output.validate()?;
        Ok(output)
    }
}

impl From<&az_physics::RayCastHit> for RayCastHit {
    fn from(value: &az_physics::RayCastHit) -> Self {
        Self {
            distance: value.distance,
            position: value.position,
            normal: value.normal,
            entity_id: value
                .entity_id
                .map_or(EntityId::INVALID, |id| EntityId::new(id.0)),
        }
    }
}

impl From<&az_physics::RayCastResult> for RayCastResult {
    fn from(value: &az_physics::RayCastResult) -> Self {
        let mut output = Self::default();
        for hit in value.piercing_hits() {
            output.add_piercing_hit(hit.into());
        }
        if let Some(hit) = value.blocking_hit() {
            output.set_blocking_hit(hit.into());
        }
        output
    }
}

/// `LmbrCentral` query facade over the backend-neutral physics world.
pub trait LmbrCentralPhysicsQueries {
    /// Cast a ray described in the `LmbrCentral` authoring vocabulary.
    ///
    /// # Errors
    ///
    /// [`az_physics::PhysicsError::InvalidRayHitCount`] if `max_hits` is
    /// negative, whatever [`az_physics::RayCastConfiguration::validate`]
    /// rejects about the ray (a non-finite origin, direction or distance), and
    /// any error the backend's own cast returns.
    fn ray_cast_lmbr(
        &self,
        configuration: &RayCastConfiguration,
    ) -> Result<RayCastResult, az_physics::PhysicsError>;

    /// Collect the entities whose bodies overlap the world-space box.
    ///
    /// # Errors
    ///
    /// Any error the backend's overlap query returns for the assembled
    /// [`az_physics::AabbQuery`], such as an inverted or non-finite box.
    fn gather_physical_entities_in_aabb(
        &self,
        min: impl Into<Vec3>,
        max: impl Into<Vec3>,
        physical_entity_types: az_physics::PhysicalEntityTypes,
        max_results: usize,
    ) -> Result<Vec<EntityId>, az_physics::PhysicsError>;
}

impl<B: az_physics::PhysicsBackend> LmbrCentralPhysicsQueries for az_physics::PhysicsScene<B> {
    fn ray_cast_lmbr(
        &self,
        configuration: &RayCastConfiguration,
    ) -> Result<RayCastResult, az_physics::PhysicsError> {
        let configuration = az_physics::RayCastConfiguration::try_from(configuration)?;
        self.ray_cast(&configuration).map(|result| (&result).into())
    }

    fn gather_physical_entities_in_aabb(
        &self,
        min: impl Into<Vec3>,
        max: impl Into<Vec3>,
        physical_entity_types: az_physics::PhysicalEntityTypes,
        max_results: usize,
    ) -> Result<Vec<EntityId>, az_physics::PhysicsError> {
        self.overlap_aabb(az_physics::AabbQuery {
            min: min.into(),
            max: max.into(),
            physical_entity_types,
            max_results,
        })
        .map(|hits| {
            hits.into_iter()
                .filter_map(|hit| hit.entity_id)
                .map(|id| EntityId::new(id.0))
                .collect()
        })
    }
}
