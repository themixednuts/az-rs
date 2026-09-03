//! Scripting components and data for `LmbrCentral`.
//!
//! Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Scripting`.
mod random_timed_spawner;
mod spawner;
mod tag;
mod trigger_area;

use bevy::prelude::*;

pub use random_timed_spawner::*;
pub use spawner::*;
pub use tag::*;
pub use trigger_area::*;

pub fn register_scripting_components(app: &mut App) {
    app.register_type::<TriggerAreaActivationEntityType>()
        .register_type::<az_asset::UntypedAssetRef>()
        .register_type::<az_core::math::RandomDistributionType>()
        .register_type::<RandomTimedSpawnerConfiguration>()
        .register_type::<RandomTimedSpawnerComponent>()
        .register_type::<SpawnerComponent>()
        .register_type::<Vec<u64>>()
        .register_type::<Vec<TriggerAreaTag>>()
        .register_type::<TriggerAreaTag>()
        .register_type::<TriggerAreaComponent>()
        .register_type_data::<TriggerAreaComponent, az_core::ReflectAzTypeInfo>()
        .register_type_data::<TriggerAreaComponent, az_core::ReflectAzRtti>()
        .register_type::<TagComponent>();
}
