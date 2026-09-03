use std::{fs, time::Duration};

use az_scene::{
    AzSceneAsset, AzSceneEntityMetadata, AzSceneMetadata, AzScenePlugin,
    AzSceneSourceScopeMetadata, LocalEntityScopeId, encode_scene_asset,
};
use bevy::{
    app::{App, TaskPoolPlugin},
    asset::{
        Asset, AssetApp, AssetLoader, AssetPlugin, AssetServer, Assets, Handle, LoadContext,
        io::Reader,
    },
    ecs::{
        component::Component,
        reflect::{AppTypeRegistry, ReflectComponent},
        world::World,
    },
    reflect::{Reflect, TypePath},
    world_serialization::DynamicWorldBuilder,
};

#[derive(Asset, Reflect, Debug, Clone, PartialEq, Eq)]
struct DependencyAsset(Vec<u8>);

#[derive(Default, TypePath)]
struct DependencyAssetLoader;

impl AssetLoader for DependencyAssetLoader {
    type Asset = DependencyAsset;
    type Settings = ();
    type Error = std::io::Error;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(DependencyAsset(bytes))
    }

    fn extensions(&self) -> &[&str] {
        &["dep"]
    }
}

#[derive(Component, Reflect, Debug, Clone)]
#[reflect(Component)]
struct AssetLink {
    asset: Handle<DependencyAsset>,
}

#[test]
fn asset_handle_round_trip_uses_load_context_and_records_the_exact_ledger() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("deps")).unwrap();
    fs::write(root.path().join("deps/example.dep"), b"dependency payload").unwrap();

    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin {
            file_path: root.path().to_string_lossy().into_owned(),
            watch_for_changes_override: Some(false),
            use_asset_processor_override: Some(false),
            ..Default::default()
        },
        AzScenePlugin,
    ))
    .init_asset::<DependencyAsset>()
    .register_asset_loader(DependencyAssetLoader)
    .register_asset_reflect::<DependencyAsset>()
    .register_type::<AssetLink>();
    {
        let app_registry = app.world().resource::<AppTypeRegistry>();
        app_registry.write().register::<bevy::ecs::entity::Entity>();
    }

    let asset_server = app.world().resource::<AssetServer>().clone();
    let dependency: Handle<DependencyAsset> = asset_server.load("deps/example.dep");
    let mut source = World::new();
    let source_entity = source
        .spawn(AssetLink {
            asset: dependency.clone(),
        })
        .id();
    let asset = {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        AzSceneAsset::new_in_entity_order(
            DynamicWorldBuilder::from_world(&source, &registry)
                .extract_entity(source_entity)
                .build(),
            &[source_entity],
            AzSceneMetadata {
                source_scopes: vec![AzSceneSourceScopeMetadata { parent: None }],
                entities: vec![AzSceneEntityMetadata {
                    source_alias: "asset-user".to_owned(),
                    source_scope: LocalEntityScopeId::ROOT,
                    source_entity_id: None,
                    parent: None,
                    component_targets: Vec::new(),
                }],
            },
        )
        .unwrap()
    };
    let encoded = {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        encode_scene_asset(&asset, &registry).unwrap()
    };
    assert_eq!(encoded.dependencies, vec!["deps/example.dep"]);
    fs::write(root.path().join("roundtrip.scn.bin"), &encoded.bytes).unwrap();

    let scene_handle: Handle<AzSceneAsset> = asset_server.load("roundtrip.scn.bin");
    for _ in 0..2_000 {
        app.update();
        if app
            .world()
            .resource::<Assets<AzSceneAsset>>()
            .get(scene_handle.id())
            .is_some()
            && app
                .world()
                .resource::<Assets<DependencyAsset>>()
                .get(dependency.id())
                .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    let scenes = app.world().resource::<Assets<AzSceneAsset>>();
    let loaded = scenes
        .get(scene_handle.id())
        .expect("scene loaded through AssetServer");
    assert_eq!(loaded.dependencies, vec!["deps/example.dep"]);
    let mut destination = World::new();
    let instance = {
        let registry = app.world().resource::<AppTypeRegistry>().read();
        loaded.materialize(&mut destination, &registry).unwrap()
    };
    let link = destination
        .get::<AssetLink>(instance.entities[0])
        .expect("typed handle component");
    assert_eq!(link.asset.path().unwrap().to_string(), "deps/example.dep");
    assert_eq!(link.asset.id(), dependency.id());
    assert_eq!(
        app.world()
            .resource::<Assets<DependencyAsset>>()
            .get(link.asset.id()),
        Some(&DependencyAsset(b"dependency payload".to_vec()))
    );
}
