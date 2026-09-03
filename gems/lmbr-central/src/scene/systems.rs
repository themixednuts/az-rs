use az_mesh_builder::MeshAsset;
use bevy::prelude::*;
use bevy::world_serialization::DynamicWorldRoot;

use super::binding::SceneAssetBinding;
use super::resolver::resolve_scene_asset_path;
use super::source::SceneComponentSource;
use crate::{
    InstancedMeshChildren, InstancedMeshComponent, InstancedMeshInstance, MaterialAsset,
    MeshComponent, StaticModelBinding, StaticModelChildren, despawn_static_model_children,
    is_static_model_engine_asset_path, is_static_model_source_asset_path,
    material_asset_path_from_component, static_model_engine_asset_path,
};

/// Syncs renderable component sources into Bevy asset bindings.
#[allow(clippy::type_complexity)]
#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub fn sync_scene_components<T: Component + SceneComponentSource>(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    static_model_assets: Option<Res<Assets<MeshAsset>>>,
    material_assets: Option<Res<Assets<MaterialAsset>>>,
    query: Query<(
        Entity,
        Ref<T>,
        Option<&DynamicWorldRoot>,
        Option<&SceneAssetBinding>,
        Option<&StaticModelBinding>,
        Option<&StaticModelChildren>,
        Option<&Transform>,
        Option<&Name>,
    )>,
) {
    let asset_server = asset_server.as_deref();
    let static_model_assets = static_model_assets.as_deref();
    let material_assets = material_assets.as_deref();
    for (
        entity,
        component,
        scene_root,
        binding,
        static_model,
        static_model_children,
        transform,
        name,
    ) in &query
    {
        if !should_sync_scene_component(
            component.is_changed(),
            asset_server,
            scene_root,
            binding,
            static_model,
            transform,
            name,
        ) {
            continue;
        }

        sync_scene_component(
            &mut commands,
            asset_server,
            entity,
            component.scene_asset_path(),
            component.material_override_asset_path(),
            component.visible(),
            scene_root,
            static_model_assets,
            material_assets,
            static_model,
            static_model_children,
            transform,
            name,
            T::DEFAULT_NAME,
        );
    }
}

#[allow(clippy::type_complexity)]
pub fn sync_instanced_mesh_components(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            Ref<InstancedMeshComponent>,
            Option<&InstancedMeshChildren>,
            Option<&DynamicWorldRoot>,
            Option<&SceneAssetBinding>,
            Option<&StaticModelBinding>,
            Option<&StaticModelChildren>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(
            Changed<InstancedMeshComponent>,
            Without<InstancedMeshChildren>,
            Without<Transform>,
            Without<Name>,
        )>,
    >,
) {
    for (
        entity,
        component,
        instance_children,
        scene_root,
        binding,
        static_model,
        static_model_children,
        transform,
        name,
    ) in &query
    {
        let mut remove_static_children = false;
        {
            let mut entity_commands = commands.entity(entity);
            if scene_root.is_some() {
                entity_commands.remove::<DynamicWorldRoot>();
            }
            if binding.is_some() {
                entity_commands.remove::<SceneAssetBinding>();
            }
            if static_model.is_some() {
                entity_commands.remove::<StaticModelBinding>();
            }
            if static_model_children.is_some() {
                entity_commands.remove::<StaticModelChildren>();
                remove_static_children = true;
            }
            entity_commands.insert(if component.render_node.mesh.visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
            if transform.is_none() {
                entity_commands.insert(Transform::default());
            }
            if name.is_none() {
                entity_commands.insert(Name::new("InstancedMeshComponent"));
            }
        }

        if remove_static_children {
            despawn_static_model_children(&mut commands, static_model_children);
        }

        if component.is_changed() || instance_children.is_none() {
            despawn_instanced_mesh_children(&mut commands, instance_children);
            spawn_instanced_mesh_children(&mut commands, entity, &component);
        }
    }
}

pub fn cleanup_removed_instanced_mesh_components(
    mut commands: Commands,
    mut removed: RemovedComponents<InstancedMeshComponent>,
    children: Query<&InstancedMeshChildren>,
) {
    for entity in removed.read() {
        if let Ok(children) = children.get(entity) {
            despawn_instanced_mesh_children(&mut commands, Some(children));
            commands.entity(entity).remove::<InstancedMeshChildren>();
        }
    }
}

fn spawn_instanced_mesh_children(
    commands: &mut Commands,
    entity: Entity,
    component: &InstancedMeshComponent,
) {
    let mut child_entities = Vec::with_capacity(component.instance_transforms().len().max(1));
    let mesh_component = MeshComponent {
        render_node: component.render_node.mesh.clone(),
        ..Default::default()
    };

    commands.entity(entity).with_children(|parent| {
        if component.instance_transforms().is_empty() {
            let child = parent
                .spawn((
                    Name::new("Instanced Mesh Instance"),
                    InstancedMeshInstance,
                    mesh_component,
                    Transform::default(),
                ))
                .id();
            child_entities.push(child);
            return;
        }

        for (index, transform) in component.instance_transforms().iter().enumerate() {
            let child = parent
                .spawn((
                    Name::new(format!("Instanced Mesh Instance {index}")),
                    InstancedMeshInstance,
                    mesh_component.clone(),
                    *transform,
                ))
                .id();
            child_entities.push(child);
        }
    });
    commands
        .entity(entity)
        .insert(InstancedMeshChildren(child_entities));
}

fn despawn_instanced_mesh_children(
    commands: &mut Commands,
    children: Option<&InstancedMeshChildren>,
) {
    let Some(children) = children else {
        return;
    };
    for child in &children.0 {
        commands.entity(*child).despawn();
    }
}

#[allow(clippy::too_many_arguments)]
fn should_sync_scene_component(
    component_changed: bool,
    asset_server: Option<&AssetServer>,
    scene_root: Option<&DynamicWorldRoot>,
    binding: Option<&SceneAssetBinding>,
    static_model: Option<&StaticModelBinding>,
    transform: Option<&Transform>,
    name: Option<&Name>,
) -> bool {
    component_changed
        || binding.is_none()
        || transform.is_none()
        || name.is_none()
        || (asset_server.is_some()
            && scene_root.is_none()
            && static_model.is_none()
            && binding.and_then(SceneAssetBinding::engine_path).is_some())
}

#[allow(clippy::too_many_arguments)]
fn sync_scene_component(
    commands: &mut Commands,
    asset_server: Option<&AssetServer>,
    entity: Entity,
    scene_asset_path: Option<&str>,
    material_override_asset_path: Option<&str>,
    visible: bool,
    scene_root: Option<&DynamicWorldRoot>,
    static_model_assets: Option<&Assets<MeshAsset>>,
    material_assets: Option<&Assets<MaterialAsset>>,
    static_model: Option<&StaticModelBinding>,
    static_model_children: Option<&StaticModelChildren>,
    transform: Option<&Transform>,
    name: Option<&Name>,
    default_name: &'static str,
) {
    let engine_path = scene_asset_path
        .and_then(resolve_scene_asset_path)
        .map(|path| {
            if is_static_model_source_asset_path(&path) {
                static_model_engine_asset_path(&path)
            } else {
                path
            }
        });
    let mut remove_static_children = false;

    {
        let mut entity_commands = commands.entity(entity);
        if let Some(engine_path) = engine_path.as_ref() {
            if is_static_model_engine_asset_path(engine_path) {
                if let (Some(asset_server), true) = (asset_server, static_model_assets.is_some()) {
                    if static_model.is_none_or(|binding| {
                        binding.engine_path() != engine_path
                            || binding.material_override_path() != material_override_asset_path
                    }) {
                        remove_static_children = true;
                    }
                    let material_override_path =
                        material_override_asset_path.and_then(material_asset_path_from_component);
                    let material_override = material_override_path
                        .as_ref()
                        .filter(|_| material_assets.is_some())
                        .map(|path| asset_server.load(path.clone()));
                    entity_commands.insert(
                        StaticModelBinding::new(
                            engine_path.clone(),
                            asset_server.load(engine_path.clone()),
                        )
                        .with_material_override(material_override_path, material_override),
                    );
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
            } else if let Some(asset_server) = asset_server {
                entity_commands.insert(DynamicWorldRoot(asset_server.load(engine_path.clone())));
                entity_commands.remove::<StaticModelBinding>();
                entity_commands.remove::<StaticModelChildren>();
                remove_static_children = true;
                entity_commands.insert(SceneAssetBinding::new(Some(engine_path.clone())));
            } else if scene_root.is_some() {
                entity_commands.remove::<DynamicWorldRoot>();
                entity_commands.insert(SceneAssetBinding::new(Some(engine_path.clone())));
            } else {
                entity_commands.insert(SceneAssetBinding::new(Some(engine_path.clone())));
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

        entity_commands.insert(if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        });

        if transform.is_none() {
            entity_commands.insert(Transform::default());
        }
        if name.is_none() {
            entity_commands.insert(Name::new(default_name));
        }
    }

    if remove_static_children {
        despawn_static_model_children(commands, static_model_children);
    }
}
