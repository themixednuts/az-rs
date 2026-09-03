use bevy::prelude::*;
use bevy::world_serialization::DynamicWorldRoot;

use super::{SceneAssetBinding, resolve_scene_asset_path, resolve_scene_asset_path_with_variant};
use crate::StaticModelBinding;
use crate::{
    InstancedMeshChildren, InstancedMeshComponent, InstancedMeshComponentRenderNode,
    InstancedMeshInstance, LmbrCentralPlugin, MeshComponent, MeshComponentRenderNode,
};

#[test]
fn resolve_scene_asset_path_is_identity_on_pak_source() {
    // Identity-on-pak-source: with `engine_map` gone, the resolver
    // only normalises slashes + case and rejects empty input.
    assert_eq!(
        resolve_scene_asset_path("Objects/Settlement/House.cgf").as_deref(),
        Some("objects/settlement/house.cgf"),
    );
    assert_eq!(
        resolve_scene_asset_path("Characters/Hero.skin").as_deref(),
        Some("characters/hero.skin"),
    );
    assert_eq!(
        resolve_scene_asset_path("slices/gatherables/master_tree.dynamicslice").as_deref(),
        Some("slices/gatherables/master_tree.dynamicslice"),
    );
    assert_eq!(resolve_scene_asset_path("").as_deref(), None);
}

#[test]
fn resolve_with_variant_produces_synthetic_slice_meta_path() {
    assert_eq!(
        resolve_scene_asset_path_with_variant(
            "slices/gatherables/master_tree.dynamicslice",
            Some("OakTree_a"),
        )
        .as_deref(),
        Some("slices/gatherables/master_tree_oaktree_a.slice.meta"),
    );
}

// `plugin_refreshes_mesh_asset_binding_when_resolver_changes` was
// removed when `LmbrCentralSceneAssetResolver` was deleted. The
// resolver was a Bevy resource that retriggered sync via
// `is_changed()`; now path resolution is identity-on-source-path and
// the only re-sync trigger is the component itself changing.

#[test]
fn plugin_syncs_mesh_component_visibility_and_transform() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(MeshComponent {
            render_node: MeshComponentRenderNode {
                visible: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Hidden));
    assert!(entity_ref.contains::<Transform>());
    assert_eq!(entity_ref.get::<Name>().unwrap().as_str(), "MeshComponent");
}

#[test]
fn plugin_removes_mesh_scene_root_when_asset_path_no_longer_resolves() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        // The spawn below allocates a `DynamicWorld` handle, and a handle
        // cannot be allocated for an asset type the app never initialized.
        .init_asset::<DynamicWorld>()
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn((
            MeshComponent {
                render_node: MeshComponentRenderNode {
                    // Resolution is identity-on-source, so a path fails to
                    // resolve only by being empty; `Materials/Foo.mtl` used to
                    // stand for "unresolvable" and now resolves to itself.
                    static_mesh_asset_path: Some("   ".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            DynamicWorldRoot(Handle::<DynamicWorld>::default()),
        ))
        .id();

    app.update();

    assert!(!app.world().entity(entity).contains::<DynamicWorldRoot>());
}

#[test]
fn plugin_binds_static_mesh_components_to_static_model_assets() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<crate::NativeMeshAsset>()
        .init_asset::<crate::MaterialAsset>()
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(MeshComponent {
            render_node: MeshComponentRenderNode {
                static_mesh_asset_path: Some("Objects/Foo.cgf".to_string()),
                ..Default::default()
            },
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(!entity_ref.contains::<DynamicWorldRoot>());
    assert_eq!(
        entity_ref.get::<SceneAssetBinding>().unwrap().engine_path(),
        Some("objects/foo.azmesh")
    );
    assert_eq!(
        entity_ref
            .get::<StaticModelBinding>()
            .map(StaticModelBinding::engine_path),
        Some("objects/foo.azmesh")
    );
}

#[test]
fn plugin_binds_vanilla_static_model_paths_directly() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<crate::NativeMeshAsset>()
        .init_asset::<crate::MaterialAsset>()
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(MeshComponent {
            render_node: MeshComponentRenderNode {
                static_mesh_asset_path: Some("Objects/Foo.cgf".to_string()),
                material_override_asset_path: Some("Materials/Objects/Foo.mtl".to_string()),
                ..Default::default()
            },
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert_eq!(
        entity_ref.get::<SceneAssetBinding>().unwrap().engine_path(),
        Some("objects/foo.azmesh")
    );
    assert_eq!(
        entity_ref
            .get::<StaticModelBinding>()
            .map(StaticModelBinding::engine_path),
        Some("objects/foo.azmesh")
    );
    assert_eq!(
        entity_ref
            .get::<StaticModelBinding>()
            .and_then(StaticModelBinding::material_override_path),
        Some("materials/objects/foo.mtl")
    );
    assert!(
        entity_ref
            .get::<StaticModelBinding>()
            .and_then(StaticModelBinding::material_override)
            .is_some()
    );
}

#[test]
fn plugin_expands_instanced_mesh_components_to_mesh_instances() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<crate::NativeMeshAsset>()
        .init_asset::<crate::MaterialAsset>()
        .add_plugins(LmbrCentralPlugin);

    let first_transform = Transform::from_xyz(1.0, 2.0, 3.0);
    let second_transform = Transform::from_xyz(4.0, 5.0, 6.0);
    let entity = app
        .world_mut()
        .spawn(InstancedMeshComponent {
            render_node: InstancedMeshComponentRenderNode {
                mesh: MeshComponentRenderNode {
                    static_mesh_asset_path: Some("Objects/Foo.cgf".to_string()),
                    ..Default::default()
                },
                instance_transforms: vec![first_transform, second_transform],
            },
        })
        .id();

    app.update();

    let children = app
        .world()
        .entity(entity)
        .get::<InstancedMeshChildren>()
        .unwrap()
        .0
        .clone();
    assert_eq!(children.len(), 2);
    assert!(!app.world().entity(entity).contains::<StaticModelBinding>());

    let first_ref = app.world().entity(children[0]);
    assert!(first_ref.contains::<InstancedMeshInstance>());
    assert!(first_ref.contains::<MeshComponent>());
    assert_eq!(first_ref.get::<Transform>(), Some(&first_transform));

    let second_ref = app.world().entity(children[1]);
    assert_eq!(second_ref.get::<Transform>(), Some(&second_transform));

    app.update();

    for child in children {
        assert_eq!(
            app.world()
                .entity(child)
                .get::<StaticModelBinding>()
                .map(StaticModelBinding::engine_path),
            Some("objects/foo.azmesh")
        );
    }
}
