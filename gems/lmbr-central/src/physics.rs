//! Physics components for `LmbrCentral`.
//!
//! Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Physics/PhysicsSystemComponent.cpp`.

mod authoring;
mod character;
mod collider;
mod force_volume;
mod ids;
mod query;
mod rigid;
mod static_body;
mod system;
mod vegetation;

#[cfg(test)]
mod tests;

use bevy::prelude::*;

pub use authoring::{
    LmbrCentralAuthoringSet, LmbrCentralPhysicsError, LocalTriggerEntity, PhysicsEnabled,
    RecordedPhysicsCollisions, TriggerAreaEvent, TriggerAreaState,
};
pub use character::{
    CHARACTER_PHYSICS_COMPONENT_TYPE_ID, CRY_PLAYER_PHYSICS_CONFIGURATION_TYPE_ID,
    CharacterPhysicsComponent, CryPlayerPhysicsConfiguration, PLAYER_DIMENSIONS_TYPE_ID,
    PLAYER_DYNAMICS_TYPE_ID, PlayerDimensions, PlayerDynamics,
};
pub use collider::{MeshColliderComponent, PrimitiveColliderComponent, PrimitiveColliderConfig};
pub use force_volume::{
    ForceMode, ForceSpace, ForceVolumeComponent, ForceVolumeConfiguration, UnknownForceMode,
    UnknownForceSpace,
};
pub use ids::*;
pub use query::{
    LmbrCentralPhysicsQueries, RAY_CAST_CONFIGURATION_TYPE_ID, RAY_CAST_HIT_TYPE_ID,
    RAY_CAST_RESULT_TYPE_ID, RayCastConfiguration, RayCastHit, RayCastResult,
};
pub use rigid::{MassOrDensity, PhysicsComponent, RigidPhysicsComponent, RigidPhysicsConfig};
pub use static_body::{StaticPhysicsComponent, StaticPhysicsConfig};
pub use system::PhysicsSystemComponent;
pub use vegetation::{VEGETATION_PHYSICS_SERVICE, VegetationPhysicsComponent};

pub fn register_physics_components(app: &mut App) {
    app.register_type::<PhysicsSystemComponent>()
        .register_type::<PlayerDimensions>()
        .register_type_data::<PlayerDimensions, az_core::ReflectAzTypeInfo>()
        .register_type::<PlayerDynamics>()
        .register_type_data::<PlayerDynamics, az_core::ReflectAzTypeInfo>()
        .register_type::<CryPlayerPhysicsConfiguration>()
        .register_type_data::<CryPlayerPhysicsConfiguration, az_core::ReflectAzTypeInfo>()
        .register_type::<CharacterPhysicsComponent>()
        .register_type_data::<CharacterPhysicsComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<CharacterPhysicsComponent, az_core::ReflectAzRtti>()
        .register_type::<RayCastConfiguration>()
        .register_type_data::<RayCastConfiguration, az_core::ReflectAzTypeInfo>()
        .register_type::<RayCastHit>()
        .register_type_data::<RayCastHit, az_core::ReflectAzTypeInfo>()
        .register_type::<RayCastResult>()
        .register_type_data::<RayCastResult, az_core::ReflectAzTypeInfo>()
        .register_type::<PrimitiveColliderConfig>()
        .register_type_data::<PrimitiveColliderConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<PrimitiveColliderConfig, az_core::ReflectAzRtti>()
        .register_type::<PrimitiveColliderComponent>()
        .register_type_data::<PrimitiveColliderComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<PrimitiveColliderComponent, az_core::ReflectAzRtti>()
        .register_type::<MeshColliderComponent>()
        .register_type_data::<MeshColliderComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<MeshColliderComponent, az_core::ReflectAzRtti>()
        .register_type::<MassOrDensity>()
        .register_type_data::<MassOrDensity, az_core::ReflectAzTypeInfo>()
        .register_type::<PhysicsComponent>()
        .register_type_data::<PhysicsComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<PhysicsComponent, az_core::ReflectAzRtti>()
        .register_type::<RigidPhysicsConfig>()
        .register_type_data::<RigidPhysicsConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<RigidPhysicsConfig, az_core::ReflectAzRtti>()
        .register_type::<RigidPhysicsComponent>()
        .register_type_data::<RigidPhysicsComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<RigidPhysicsComponent, az_core::ReflectAzRtti>()
        .register_type::<StaticPhysicsConfig>()
        .register_type_data::<StaticPhysicsConfig, az_core::ReflectAzTypeInfo>()
        .register_type_data::<StaticPhysicsConfig, az_core::ReflectAzRtti>()
        .register_type::<StaticPhysicsComponent>()
        .register_type_data::<StaticPhysicsComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<StaticPhysicsComponent, az_core::ReflectAzRtti>()
        .register_type::<ForceVolumeConfiguration>()
        .register_type_data::<ForceVolumeConfiguration, az_core::ReflectAzTypeInfo>()
        .register_type::<ForceMode>()
        .register_type::<ForceSpace>()
        .register_type::<ForceVolumeComponent>()
        .register_type_data::<ForceVolumeComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<ForceVolumeComponent, az_core::ReflectAzRtti>()
        .register_type::<VegetationPhysicsComponent>()
        .register_type_data::<VegetationPhysicsComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<VegetationPhysicsComponent, az_core::ReflectAzRtti>();
    authoring::configure(app);
}
