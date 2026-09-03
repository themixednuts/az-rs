//! Reflected Cry living-entity component data.

use az_core::component::Component as AzComponent;
use az_derive::{AzRtti, AzTypeInfo};
use az_physics::{LivingBodyConfiguration, LivingDimensions, LivingDynamics, SurfaceIndex};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

pub const PLAYER_DIMENSIONS_TYPE_ID: Uuid = uuid!("64B9DBDA-90D4-4D3D-88EA-36810AF2C98F");
pub const PLAYER_DYNAMICS_TYPE_ID: Uuid = uuid!("B1237004-14E1-4327-8774-D2C5796230E7");
pub const CRY_PLAYER_PHYSICS_CONFIGURATION_TYPE_ID: Uuid =
    uuid!("97A1C07E-0444-4FAC-A394-8317AFE5696B");
pub const CHARACTER_PHYSICS_COMPONENT_TYPE_ID: Uuid = uuid!("D707D6C5-3EFA-4275-82EB-A954F845D324");

/// Serialized `PlayerDimensions` version 1 data.
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "PlayerDimensions", PLAYER_DIMENSIONS_TYPE_ID)]
pub struct PlayerDimensions {
    #[serde(rename = "Use Capsule", default)]
    pub use_capsule: bool,
    #[serde(rename = "Collider Radius", default)]
    pub collider_radius: f32,
    #[serde(rename = "Collider Half-Height", default)]
    pub collider_half_height: f32,
    #[serde(rename = "Height Collider", default)]
    pub height_collider: f32,
    #[serde(rename = "Height Pivot", default)]
    pub height_pivot: f32,
    #[serde(rename = "Height Eye", default)]
    pub height_eye: f32,
    #[serde(rename = "Height Head", default)]
    pub height_head: f32,
    #[serde(rename = "Head Radius", default)]
    pub head_radius: f32,
    #[serde(rename = "Unprojection Direction", default)]
    pub unprojection_direction: Vec3,
    #[serde(rename = "Max Unprojection", default)]
    pub max_unprojection: f32,
    #[serde(rename = "Ground Contact Epsilon", default)]
    pub ground_contact_epsilon: f32,
}

impl Default for PlayerDimensions {
    fn default() -> Self {
        LivingDimensions::default().into()
    }
}

impl From<PlayerDimensions> for LivingDimensions {
    fn from(value: PlayerDimensions) -> Self {
        Self {
            use_capsule: value.use_capsule,
            collider_radius: value.collider_radius,
            collider_half_height: value.collider_half_height,
            height_collider: value.height_collider,
            height_pivot: value.height_pivot,
            height_eye: value.height_eye,
            height_head: value.height_head,
            head_radius: value.head_radius,
            unprojection_direction: value.unprojection_direction,
            max_unprojection: value.max_unprojection,
            ground_contact_epsilon: value.ground_contact_epsilon,
        }
    }
}

impl From<LivingDimensions> for PlayerDimensions {
    fn from(value: LivingDimensions) -> Self {
        Self {
            use_capsule: value.use_capsule,
            collider_radius: value.collider_radius,
            collider_half_height: value.collider_half_height,
            height_collider: value.height_collider,
            height_pivot: value.height_pivot,
            height_eye: value.height_eye,
            height_head: value.height_head,
            head_radius: value.head_radius,
            unprojection_direction: value.unprojection_direction,
            max_unprojection: value.max_unprojection,
            ground_contact_epsilon: value.ground_contact_epsilon,
        }
    }
}

/// Serialized `PlayerDynamics` version 1 data.
#[allow(
    clippy::struct_excessive_bools,
    reason = "the fields are the direct reflected Cry configuration"
)]
#[derive(AzTypeInfo, Debug, Clone, Copy, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "PlayerDynamics", PLAYER_DYNAMICS_TYPE_ID)]
pub struct PlayerDynamics {
    #[serde(rename = "Mass", default)]
    pub mass: f32,
    #[serde(rename = "Inertia", default)]
    pub inertia: f32,
    #[serde(rename = "Inertia Acceleration", default)]
    pub inertia_acceleration: f32,
    #[serde(rename = "Time Impulse Recover", default)]
    pub time_impulse_recover: f32,
    #[serde(rename = "Air Control", default)]
    pub air_control: f32,
    #[serde(rename = "Air Resistance", default)]
    pub air_resistance: f32,
    #[serde(rename = "Use Custom Gravity", default)]
    pub use_custom_gravity: bool,
    #[serde(rename = "Gravity", default)]
    pub gravity: Vec3,
    #[serde(rename = "Nod Speed", default)]
    pub nod_speed: f32,
    #[serde(rename = "Is Active", default)]
    pub is_active: bool,
    #[serde(rename = "Release Ground Collider When Not Active", default)]
    pub release_ground_collider_when_not_active: bool,
    #[serde(rename = "Is Swimming", default)]
    pub is_swimming: bool,
    #[serde(rename = "Surface Index", default)]
    pub surface_index: i32,
    #[serde(rename = "Min Fall Angle", default)]
    pub min_fall_angle: f32,
    #[serde(rename = "Min Slide Angle", default)]
    pub min_slide_angle: f32,
    #[serde(rename = "Max Climb Angle", default)]
    pub max_climb_angle: f32,
    #[serde(rename = "Max Jump Angle", default)]
    pub max_jump_angle: f32,
    #[serde(rename = "Max Velocity Ground", default)]
    pub max_velocity_ground: f32,
    #[serde(rename = "Collide With Terrain", default)]
    pub collide_with_terrain: bool,
    #[serde(rename = "Collide With Static", default)]
    pub collide_with_static: bool,
    #[serde(rename = "Collide With Rigid", default)]
    pub collide_with_rigid: bool,
    #[serde(rename = "Collide With Sleeping Rigid", default)]
    pub collide_with_sleeping_rigid: bool,
    #[serde(rename = "Collide With Living", default)]
    pub collide_with_living: bool,
    #[serde(rename = "Collide With Independent", default)]
    pub collide_with_independent: bool,
    #[serde(rename = "RecordCollisions", default)]
    pub record_collisions: bool,
    #[serde(rename = "MaxRecordedCollisions", default)]
    pub max_recorded_collisions: i32,
}

impl Default for PlayerDynamics {
    fn default() -> Self {
        LivingDynamics::default().into()
    }
}

impl From<PlayerDynamics> for LivingDynamics {
    fn from(value: PlayerDynamics) -> Self {
        Self {
            mass: value.mass,
            inertia: value.inertia,
            inertia_acceleration: value.inertia_acceleration,
            time_impulse_recover: value.time_impulse_recover,
            air_control: value.air_control,
            air_resistance: value.air_resistance,
            use_custom_gravity: value.use_custom_gravity,
            gravity: value.gravity,
            nod_speed: value.nod_speed,
            is_active: value.is_active,
            release_ground_collider_when_not_active: value.release_ground_collider_when_not_active,
            is_swimming: value.is_swimming,
            surface_index: SurfaceIndex(value.surface_index),
            min_fall_angle: value.min_fall_angle,
            min_slide_angle: value.min_slide_angle,
            max_climb_angle: value.max_climb_angle,
            max_jump_angle: value.max_jump_angle,
            max_ground_velocity: value.max_velocity_ground,
            collide_with_terrain: value.collide_with_terrain,
            collide_with_static: value.collide_with_static,
            collide_with_rigid: value.collide_with_rigid,
            collide_with_sleeping_rigid: value.collide_with_sleeping_rigid,
            collide_with_living: value.collide_with_living,
            collide_with_independent: value.collide_with_independent,
            record_collisions: value.record_collisions,
            max_recorded_collisions: value.max_recorded_collisions,
        }
    }
}

impl From<LivingDynamics> for PlayerDynamics {
    fn from(value: LivingDynamics) -> Self {
        Self {
            mass: value.mass,
            inertia: value.inertia,
            inertia_acceleration: value.inertia_acceleration,
            time_impulse_recover: value.time_impulse_recover,
            air_control: value.air_control,
            air_resistance: value.air_resistance,
            use_custom_gravity: value.use_custom_gravity,
            gravity: value.gravity,
            nod_speed: value.nod_speed,
            is_active: value.is_active,
            release_ground_collider_when_not_active: value.release_ground_collider_when_not_active,
            is_swimming: value.is_swimming,
            surface_index: value.surface_index.0,
            min_fall_angle: value.min_fall_angle,
            min_slide_angle: value.min_slide_angle,
            max_climb_angle: value.max_climb_angle,
            max_jump_angle: value.max_jump_angle,
            max_velocity_ground: value.max_ground_velocity,
            collide_with_terrain: value.collide_with_terrain,
            collide_with_static: value.collide_with_static,
            collide_with_rigid: value.collide_with_rigid,
            collide_with_sleeping_rigid: value.collide_with_sleeping_rigid,
            collide_with_living: value.collide_with_living,
            collide_with_independent: value.collide_with_independent,
            record_collisions: value.record_collisions,
            max_recorded_collisions: value.max_recorded_collisions,
        }
    }
}

/// Serialized `CryPlayerPhysicsConfiguration` version 1 data.
#[derive(AzTypeInfo, Debug, Clone, Copy, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(
    name = "CryPlayerPhysicsConfiguration",
    CRY_PLAYER_PHYSICS_CONFIGURATION_TYPE_ID
)]
pub struct CryPlayerPhysicsConfiguration {
    #[serde(rename = "Player Dimensions", default)]
    pub player_dimensions: PlayerDimensions,
    #[serde(rename = "Player Dynamics", default)]
    pub player_dynamics: PlayerDynamics,
}

impl From<CryPlayerPhysicsConfiguration> for LivingBodyConfiguration {
    fn from(value: CryPlayerPhysicsConfiguration) -> Self {
        Self {
            dimensions: value.player_dimensions.into(),
            dynamics: value.player_dynamics.into(),
            ..Self::default()
        }
    }
}

/// Runtime `CharacterPhysicsComponent` version 1.
#[derive(
    AzRtti,
    Component,
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Reflect,
    Serialize,
    Deserialize,
    Prefab,
)]
#[az_rtti(
    name = "CharacterPhysicsComponent",
    CHARACTER_PHYSICS_COMPONENT_TYPE_ID,
    az_core::component::Component,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
#[prefab(tag = "azoth.lmbr_central.CharacterPhysicsComponent", version = 1)]
pub struct CharacterPhysicsComponent {
    #[serde(rename = "BaseClass1", default)]
    #[reflect(ignore)]
    pub az_component: AzComponent,
    #[serde(rename = "Configuration", default)]
    pub configuration: CryPlayerPhysicsConfiguration,
}

impl CharacterPhysicsComponent {
    #[must_use]
    pub fn living_body_configuration(&self) -> LivingBodyConfiguration {
        self.configuration.into()
    }
}
