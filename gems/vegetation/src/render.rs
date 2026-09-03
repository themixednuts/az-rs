//! Vegetation instance rendering and scene binding.

mod fallback;
mod systems;

use az_gem_lmbr_central::SceneAssetBinding;
use bevy::prelude::*;

use fallback::VegetationFallbackRenderAssets;
use systems::{
    sync_instance_fallback_rendering, sync_instance_scene_roots, sync_instance_transforms,
};

pub use fallback::{
    DEFAULT_FALLBACK_INSTANCE_ROUGHNESS, DEFAULT_FALLBACK_INSTANCE_SIZE, VegetationFallbackRender,
    VegetationRenderConfig,
};

pub fn register_render_components(app: &mut App) {
    app.init_resource::<VegetationRenderConfig>()
        .init_resource::<VegetationFallbackRenderAssets>()
        .register_type::<VegetationRenderConfig>()
        .register_type::<VegetationFallbackRender>()
        .register_type::<SceneAssetBinding>()
        .add_systems(
            Update,
            (
                sync_instance_transforms,
                sync_instance_scene_roots,
                sync_instance_fallback_rendering,
            )
                .chain(),
        );
}

#[cfg(test)]
mod tests;
