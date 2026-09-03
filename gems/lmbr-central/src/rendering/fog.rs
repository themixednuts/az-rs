//! Fog volume component data and Bevy sync.

use bevy::prelude::*;

mod component;
mod config;
mod systems;
mod type_ids;
mod volume_type;

use systems::sync_fog_volume_components;

pub use component::FogVolumeComponent;
pub use config::FogVolumeConfiguration;
pub use type_ids::{FOG_VOLUME_COMPONENT_TYPE_ID, FOG_VOLUME_CONFIGURATION_TYPE_ID};
pub use volume_type::FogVolumeType;

pub(super) fn register_fog_volume_components(app: &mut App) {
    app.register_type::<FogVolumeType>()
        .register_type::<FogVolumeConfiguration>()
        .register_type::<FogVolumeComponent>()
        .add_systems(Update, sync_fog_volume_components);
}
