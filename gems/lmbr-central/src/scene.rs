//! Scene asset binding and sync systems for `LmbrCentral` mesh components.

mod binding;
mod resolver;
mod source;
mod systems;

#[cfg(test)]
mod tests;

use bevy::prelude::*;

use crate::mesh::SkinnedMeshComponent;

pub use binding::SceneAssetBinding;
pub use resolver::{
    resolve_scene_asset_path, resolve_scene_asset_path_with_variant, slice_metadata_source_path,
    slice_metadata_source_path_with_variant,
};
pub use source::SceneComponentSource;
pub use systems::{
    cleanup_removed_instanced_mesh_components, sync_instanced_mesh_components,
    sync_scene_components,
};

pub fn register_scene_components(app: &mut App) {
    app.register_type::<SceneAssetBinding>().add_systems(
        Update,
        (
            sync_instanced_mesh_components,
            cleanup_removed_instanced_mesh_components,
            sync_scene_components::<SkinnedMeshComponent>,
        ),
    );
}
