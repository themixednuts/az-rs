use az_gem_lmbr_central::{SceneAssetBinding, StaticModelBinding};
use bevy::prelude::*;
use bevy::world_serialization::DynamicWorldRoot;

use super::scene_asset::instance_scene_asset_path;
use crate::descriptor::VegetationDescriptorListComponent;
use crate::instance::InstanceData;

use crate::render::fallback::{
    VegetationFallbackRender, VegetationFallbackRenderAssets, VegetationRenderConfig,
};

// `needless_pass_by_value`: `Res`/`ResMut`/`Query`/`Commands` are owned Bevy
// system parameters; borrowing them stops the function satisfying `IntoSystem`
// and it no longer registers as a system.
#[allow(
    clippy::too_many_arguments,
    clippy::type_complexity,
    clippy::needless_pass_by_value
)]
pub(in crate::render) fn sync_instance_fallback_rendering(
    mut commands: Commands,
    config: Res<VegetationRenderConfig>,
    asset_server: Option<Res<AssetServer>>,
    mut render_assets: ResMut<VegetationFallbackRenderAssets>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    descriptor_lists: Query<Ref<VegetationDescriptorListComponent>>,
    query: Query<
        (
            Entity,
            &InstanceData,
            Option<&Mesh3d>,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&DynamicWorldRoot>,
            Option<&StaticModelBinding>,
            Option<&SceneAssetBinding>,
        ),
        (
            With<InstanceData>,
            Or<(Without<Mesh3d>, Without<MeshMaterial3d<StandardMaterial>>)>,
        ),
    >,
) {
    let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) else {
        return;
    };

    let mut mesh_handle: Option<Handle<Mesh>> = None;
    let mut material_handle: Option<Handle<StandardMaterial>> = None;

    for (entity, instance, mesh, material, scene_root, static_model, binding) in &query {
        if scene_root.is_some() || static_model.is_some() {
            continue;
        }
        if binding.is_some_and(|binding| binding.engine_path().is_some()) {
            continue;
        }
        if asset_server.is_some()
            && instance_scene_asset_path(entity, instance, &descriptor_lists).is_some()
        {
            continue;
        }

        let mut entity_commands = commands.entity(entity);
        if mesh.is_none() {
            let handle = if let Some(handle) = &mesh_handle {
                handle.clone()
            } else {
                let handle = render_assets.mesh(meshes, &config);
                mesh_handle = Some(handle.clone());
                handle
            };
            entity_commands.insert(Mesh3d(handle));
        }
        if material.is_none() {
            let handle = if let Some(handle) = &material_handle {
                handle.clone()
            } else {
                let handle = render_assets.material(materials, &config);
                material_handle = Some(handle.clone());
                handle
            };
            entity_commands.insert(MeshMaterial3d(handle));
        }
        entity_commands.insert(VegetationFallbackRender);
    }
}
