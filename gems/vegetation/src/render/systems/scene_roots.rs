use az_gem_lmbr_central::{
    NativeMeshAsset, SceneAssetBinding, StaticModelBinding, StaticModelChildren,
    is_static_model_engine_asset_path,
};
use bevy::prelude::*;
use bevy::world_serialization::DynamicWorldRoot;

use super::scene_asset::{instance_descriptor_list_changed, instance_scene_asset_path};
use crate::descriptor::VegetationDescriptorListComponent;
use crate::instance::InstanceData;
use crate::render::fallback::VegetationFallbackRender;

// `needless_pass_by_value`: `Res`/`Query`/`Commands` are owned Bevy system
// parameters; borrowing them stops the function satisfying `IntoSystem` and it
// no longer registers as a system.
#[allow(clippy::type_complexity, clippy::needless_pass_by_value)]
pub(in crate::render) fn sync_instance_scene_roots(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    static_model_assets: Option<Res<Assets<NativeMeshAsset>>>,
    descriptor_lists: Query<Ref<VegetationDescriptorListComponent>>,
    query: Query<(
        Entity,
        Ref<InstanceData>,
        Option<&DynamicWorldRoot>,
        Option<&StaticModelBinding>,
        Option<&StaticModelChildren>,
        Option<&VegetationFallbackRender>,
        Option<&SceneAssetBinding>,
    )>,
) {
    for (
        entity,
        instance,
        scene_root,
        static_model,
        static_model_children,
        fallback_render,
        binding,
    ) in &query
    {
        let descriptor_changed =
            instance_descriptor_list_changed(entity, &instance, &descriptor_lists);
        if !instance.is_changed() && !descriptor_changed && binding.is_some() {
            continue;
        }

        let engine_path = instance_scene_asset_path(entity, &instance, &descriptor_lists);
        let mut remove_static_children = false;

        {
            let mut entity_commands = commands.entity(entity);
            if let (Some(asset_server), Some(engine_path)) =
                (asset_server.as_deref(), engine_path.as_ref())
            {
                if is_static_model_engine_asset_path(engine_path) {
                    if static_model_assets.is_some() {
                        if static_model.is_none_or(|binding| binding.engine_path() != engine_path) {
                            entity_commands.remove::<StaticModelChildren>();
                            remove_static_children = true;
                        }
                        entity_commands.insert(StaticModelBinding::new(
                            engine_path.clone(),
                            asset_server.load(engine_path.clone()),
                        ));
                        entity_commands.insert(SceneAssetBinding::new(Some(engine_path.clone())));
                    } else {
                        entity_commands.remove::<StaticModelBinding>();
                        entity_commands.remove::<StaticModelChildren>();
                        remove_static_children = true;
                        entity_commands.insert(SceneAssetBinding::new(None));
                    }
                    if scene_root.is_some() {
                        entity_commands.remove::<DynamicWorldRoot>();
                    }
                } else {
                    entity_commands
                        .insert(DynamicWorldRoot(asset_server.load(engine_path.clone())));
                    entity_commands.remove::<StaticModelBinding>();
                    entity_commands.remove::<StaticModelChildren>();
                    remove_static_children = true;
                    entity_commands.insert(SceneAssetBinding::new(Some(engine_path.clone())));
                }
                if fallback_render.is_some() {
                    entity_commands.remove::<Mesh3d>();
                    entity_commands.remove::<MeshMaterial3d<StandardMaterial>>();
                    entity_commands.remove::<VegetationFallbackRender>();
                }
            } else {
                if scene_root.is_some() {
                    entity_commands.remove::<DynamicWorldRoot>();
                }
                entity_commands.remove::<StaticModelBinding>();
                entity_commands.remove::<StaticModelChildren>();
                remove_static_children = true;
                entity_commands.insert(SceneAssetBinding::new(None));
            }
        }

        if remove_static_children {
            despawn_static_model_children(&mut commands, static_model_children);
        }
    }
}

fn despawn_static_model_children(commands: &mut Commands, children: Option<&StaticModelChildren>) {
    let Some(children) = children else {
        return;
    };
    for child in &children.0 {
        commands.entity(*child).despawn();
    }
}
