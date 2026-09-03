//! Light component data and Bevy light sync.

mod component;
mod config;
mod ids;
mod systems;

use bevy::prelude::*;

use systems::sync_light_components;

pub use component::LightComponent;
pub use config::{LightConfiguration, LightCubemapResolution, LightType, VoxelGiMode};
pub use ids::*;

pub(super) fn register_light_components(app: &mut App) {
    app.register_type::<LightType>()
        .register_type::<LightCubemapResolution>()
        .register_type::<VoxelGiMode>()
        .register_type::<LightConfiguration>()
        .register_type::<LightComponent>()
        .add_systems(Update, sync_light_components);
}
