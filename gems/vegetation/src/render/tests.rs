use super::*;
use crate::*;
use az_gem_lmbr_central::{SceneAssetBinding, StaticModelBinding};
use bevy::math::primitives::Cuboid;
use bevy::world_serialization::{DynamicWorld, DynamicWorldRoot};

#[test]
fn instance_data_syncs_to_bevy_transform() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(VegetationPlugin);

    let alignment = Quat::from_rotation_x(0.25);
    let rotation = Quat::from_rotation_y(0.5);
    let entity = app
        .world_mut()
        .spawn(InstanceData {
            instance_id: InstanceId(99),
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation,
            alignment,
            scale: 1.75,
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    let transform = entity_ref.get::<Transform>().unwrap();
    assert_eq!(transform.translation, Vec3::new(1.0, 2.0, 3.0));
    assert!(quat_close(transform.rotation, alignment * rotation));
    assert_eq!(transform.scale, Vec3::splat(1.75));
    assert_eq!(
        entity_ref.get::<Name>().unwrap().as_str(),
        "Vegetation Instance 99"
    );
}

#[test]
fn plugin_renders_instance_data_with_shared_fallback_assets() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(VegetationPlugin);

    let first = app.world_mut().spawn(InstanceData::default()).id();
    let second = app
        .world_mut()
        .spawn(InstanceData {
            position: Vec3::new(2.0, 0.0, 3.0),
            ..Default::default()
        })
        .id();

    app.update();

    let first_ref = app.world().entity(first);
    let first_mesh = first_ref.get::<Mesh3d>().unwrap().0.clone();
    let first_material = first_ref
        .get::<MeshMaterial3d<StandardMaterial>>()
        .unwrap()
        .0
        .clone();
    assert!(first_ref.contains::<VegetationFallbackRender>());

    let second_ref = app.world().entity(second);
    assert_eq!(second_ref.get::<Mesh3d>().unwrap().0, first_mesh);
    assert_eq!(
        second_ref
            .get::<MeshMaterial3d<StandardMaterial>>()
            .unwrap()
            .0,
        first_material
    );

    let material = app
        .world()
        .resource::<Assets<StandardMaterial>>()
        .get(&first_material)
        .unwrap();
    assert_eq!(
        material.base_color,
        VegetationRenderConfig::default().fallback_base_color
    );
}

#[test]
fn plugin_renders_descriptor_backed_instance_as_static_model() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<DynamicWorld>()
        .init_asset::<az_gem_lmbr_central::NativeMeshAsset>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(VegetationPlugin);

    let descriptor_entity = app
        .world_mut()
        .spawn(VegetationDescriptorListComponent {
            configuration: VegetationDescriptorListConfig {
                vegetation_descriptors: vec![VegetationDescriptor {
                    instance_spawner: InstanceSpawner::LegacyVegetation(
                        LegacyVegetationInstanceSpawner {
                            mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                }],
            },
        })
        .id();
    let entity = app
        .world_mut()
        .spawn(InstanceData {
            entity: Some(descriptor_entity),
            descriptor_index: Some(0),
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(!entity_ref.contains::<DynamicWorldRoot>());
    assert!(entity_ref.contains::<StaticModelBinding>());
    assert!(!entity_ref.contains::<Mesh3d>());
    assert!(!entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert!(!entity_ref.contains::<VegetationFallbackRender>());
    // The descriptor names the `.cgf` source; what a host binds is the
    // `.azmesh` product, exactly as `lmbr_central`'s own scene binding does.
    assert_eq!(
        entity_ref.get::<SceneAssetBinding>().unwrap().engine_path(),
        Some("objects/nature/oak.azmesh")
    );
}

#[test]
fn plugin_renders_dynamic_slice_variant_as_sidecar_scene() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<DynamicWorld>()
        .init_asset::<az_gem_lmbr_central::NativeMeshAsset>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(VegetationPlugin);

    let descriptor_entity = app
        .world_mut()
        .spawn(VegetationDescriptorListComponent {
            configuration: VegetationDescriptorListConfig {
                vegetation_descriptors: vec![VegetationDescriptor {
                    instance_spawner: InstanceSpawner::DynamicSlice(DynamicSliceInstanceSpawner {
                        slice_asset_path: Some(
                            "slices/gatherables/master_tree.dynamicslice".to_string(),
                        ),
                        slice_variant: Some("OakTree_a".to_string()),
                    }),
                    ..Default::default()
                }],
            },
        })
        .id();
    let entity = app
        .world_mut()
        .spawn(InstanceData {
            entity: Some(descriptor_entity),
            descriptor_index: Some(0),
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<DynamicWorldRoot>());
    assert_eq!(
        entity_ref.get::<SceneAssetBinding>().unwrap().engine_path(),
        Some("slices/gatherables/master_tree_oaktree_a.slice.meta")
    );
}

#[test]
fn plugin_refreshes_scene_binding_when_descriptor_list_changes() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<DynamicWorld>()
        .init_asset::<az_gem_lmbr_central::NativeMeshAsset>()
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(VegetationPlugin);

    let descriptor_entity = app
        .world_mut()
        .spawn(VegetationDescriptorListComponent {
            configuration: VegetationDescriptorListConfig {
                vegetation_descriptors: vec![VegetationDescriptor {
                    instance_spawner: InstanceSpawner::LegacyVegetation(
                        LegacyVegetationInstanceSpawner {
                            mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                }],
            },
        })
        .id();
    let entity = app
        .world_mut()
        .spawn(InstanceData {
            entity: Some(descriptor_entity),
            descriptor_index: Some(0),
            ..Default::default()
        })
        .id();

    app.update();

    app.world_mut()
        .entity_mut(descriptor_entity)
        .get_mut::<VegetationDescriptorListComponent>()
        .unwrap()
        .configuration
        .vegetation_descriptors[0]
        .instance_spawner = InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
        mesh_asset_path: Some("Objects/Nature/Pine.cgf".to_string()),
        ..Default::default()
    });
    app.update();

    let entity_ref = app.world().entity(entity);
    assert_eq!(
        entity_ref.get::<SceneAssetBinding>().unwrap().engine_path(),
        Some("objects/nature/pine.azmesh")
    );
}

#[test]
fn plugin_uses_fallback_for_descriptor_instance_without_asset_server() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(VegetationPlugin);

    let descriptor_entity = app
        .world_mut()
        .spawn(VegetationDescriptorListComponent {
            configuration: VegetationDescriptorListConfig {
                vegetation_descriptors: vec![VegetationDescriptor {
                    instance_spawner: InstanceSpawner::LegacyVegetation(
                        LegacyVegetationInstanceSpawner {
                            mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                }],
            },
        })
        .id();
    let entity = app
        .world_mut()
        .spawn(InstanceData {
            entity: Some(descriptor_entity),
            descriptor_index: Some(0),
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(!entity_ref.contains::<DynamicWorldRoot>());
    assert!(entity_ref.contains::<Mesh3d>());
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert!(entity_ref.contains::<VegetationFallbackRender>());
    assert_eq!(
        entity_ref.get::<SceneAssetBinding>().unwrap().engine_path(),
        None
    );
}

#[test]
fn plugin_keeps_existing_instance_render_assets() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(VegetationPlugin);

    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(1.0, 1.0, 1.0));
    let material = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(StandardMaterial::default());
    let entity = app
        .world_mut()
        .spawn((
            InstanceData::default(),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
        ))
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert_eq!(entity_ref.get::<Mesh3d>().unwrap().0, mesh);
    assert_eq!(
        entity_ref
            .get::<MeshMaterial3d<StandardMaterial>>()
            .unwrap()
            .0,
        material
    );
    assert!(!entity_ref.contains::<VegetationFallbackRender>());
}
