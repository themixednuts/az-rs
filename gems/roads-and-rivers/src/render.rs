//! Bevy render and material binding systems for roads and rivers.

mod binding;
mod config;
mod systems;

use az_gem_lmbr_central::MaterialAsset;
use bevy::asset::AssetEvent;
use bevy::prelude::*;

use binding::apply_material_asset_bindings;
use systems::{render_river_components, render_road_components};

pub use binding::RoadsAndRiversMaterialBinding;
pub use config::RoadsAndRiversRenderConfig;

pub fn register_render_components(app: &mut App) {
    app.register_type::<RoadsAndRiversRenderConfig>()
        .init_resource::<RoadsAndRiversRenderConfig>()
        .add_message::<AssetEvent<MaterialAsset>>()
        .add_systems(
            Update,
            (
                render_road_components,
                render_river_components,
                apply_material_asset_bindings,
            ),
        );
}

#[cfg(test)]
mod tests;
