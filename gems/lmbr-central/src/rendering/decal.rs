//! Decal component data and Bevy sync.

use bevy::asset::AssetEvent;
use bevy::prelude::*;

mod component;
mod config;
mod projection;
mod systems;
mod type_ids;

use crate::material::{MaterialAsset, apply_material_asset_bindings};
use systems::sync_decal_components;

pub use component::DecalComponent;
pub use config::DecalConfiguration;
pub use projection::DecalProjectionType;
pub use type_ids::{
    DECAL_COMPONENT_TYPE_ID, DECAL_COMPONENT_TYPE_UUID, DECAL_CONFIGURATION_TYPE_ID,
    DECAL_CONFIGURATION_TYPE_UUID,
};

pub(super) fn register_decal_components(app: &mut App) {
    app.register_type::<DecalProjectionType>()
        .register_type::<DecalConfiguration>()
        .register_type::<DecalComponent>()
        .add_message::<AssetEvent<MaterialAsset>>()
        .add_systems(
            Update,
            (sync_decal_components, apply_material_asset_bindings),
        );
}
