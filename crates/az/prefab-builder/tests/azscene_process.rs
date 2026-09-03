use std::collections::BTreeMap;

use az_prefab::{
    EntityAlias, InstanceAlias, OverrideOperation, Prefab, PrefabAssetPath, PrefabBuildError,
    PrefabCodec, PrefabDocument, PrefabEntity, PrefabInstance, ReflectPrefab, ReflectedPath,
    SparseValue, TypedOverrideAction, TypedOverrideTarget,
};
use az_prefab_builder::{
    PrefabAzSceneProcessError, engine_prefab_type_registry, process_prefab_to_azscene,
};
use az_render::{DirectionalLight, Mesh};
use az_scene::read_scene_asset_from_reader;
use az_transform::Transform;
use bevy::{
    ecs::{
        component::Component,
        hierarchy::ChildOf,
        reflect::{AppTypeRegistry, ReflectComponent},
        template::TemplateContext,
        world::World,
    },
    reflect::{Reflect, Typed, std_traits::ReflectDefault},
};
use glam::{Quat, Vec3, Vec4};

#[derive(Component, Reflect, Prefab, Default)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "NativeComponentFixture", version = 1)]
struct NativeComponentFixture {
    az_component: az_core::component::Component,
}

impl az_core::AzTypeInfo for NativeComponentFixture {
    const NAME: &'static str = "NativeComponentFixture";
    const TYPE_ID: uuid::Uuid = uuid::Uuid::from_u128(0xad68_c779_16a9_4e5d_966c_1dce_8f68_0ea4);
}

impl az_core::AzRtti for NativeComponentFixture {
    const BASE_TYPE_IDS: &'static [uuid::Uuid] =
        &[<az_core::component::Component as az_core::AzTypeInfo>::TYPE_ID];
}

impl az_core::component::AzComponent for NativeComponentFixture {
    fn component_id(&self) -> az_core::component::ComponentId {
        az_core::component::AzComponent::component_id(&self.az_component)
    }
}

/// The engine's own lowerings plus this fixture's, the way a worker hands a
/// composed set to the processor.
///
/// Composed rather than assembled: the engine's adapters come from the `types`
/// and `runtime` bundles under the asset-worker role, which is exactly what a
/// real worker composes, so this fixture exercises the path rather than
/// standing beside it.
fn lowerings() -> Vec<az_core::component::ComponentLoweringRegistration> {
    let mut composer = az_gem_contract::Composer::new(az_gem_contract::GemTargetRole::AssetWorker);
    composer
        .floor(az_engine_types::types_contribution())
        .expect("the engine floor declares no host-capability floor");
    composer
        .floor(az_engine_runtime::runtime_contribution())
        .expect("the engine floor declares no host-capability floor");
    composer
        .finalize()
        .expect("the engine composition is valid");
    let mut lowerings = az_prefab_builder::lowerings(
        composer
            .registries()
            .get::<az_core::component::ComponentLoweringRegistration>()
            .expect("the engine floor registers component lowerings"),
    );
    lowerings.push(
        az_core::component::ComponentLoweringRegistration::bevy_component::<NativeComponentFixture>(
        ),
    );
    lowerings
}

fn alias(value: &str) -> EntityAlias {
    EntityAlias::new(value).unwrap()
}

fn sparse(value: impl bevy::reflect::PartialReflect) -> SparseValue {
    SparseValue::try_new(Box::new(value)).unwrap()
}

fn encode(document: &PrefabDocument, registry: &bevy::reflect::TypeRegistry) -> String {
    PrefabCodec::new(registry)
        .unwrap()
        .encode(document)
        .unwrap()
}

fn transform_entity(position: Vec3, parent: Option<&str>) -> PrefabEntity {
    PrefabEntity {
        entity_id: None,
        parent: parent.map(alias),
        components: BTreeMap::from([(
            "Transform".to_owned(),
            sparse(Transform {
                position,
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            }),
        )]),
    }
}

// The decoded scene and the materialized world borrow from this type-registry read
// guard for the rest of the test, so it cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
#[test]
fn native_component_identity_survives_prefab_processing() {
    let app_registry = engine_prefab_type_registry();
    {
        let mut registry = app_registry.write();
        registry.register::<az_core::component::Component>();
        registry.register::<az_core::component::ComponentId>();
        registry.register::<NativeComponentFixture>();
    }
    let registry = app_registry.read();
    let component_id = az_core::component::ComponentId::new(0x1020_3040_5060_7080);
    let mut document = PrefabDocument::default();
    document
        .type_versions
        .insert("NativeComponentFixture".to_owned(), 1);
    document.entities.insert(
        alias("root"),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([(
                "NativeComponentFixture".to_owned(),
                sparse(NativeComponentFixture {
                    az_component: az_core::component::Component { id: component_id },
                }),
            )]),
        },
    );
    let source = encode(&document, &registry);
    drop(registry);

    let compiled = process_prefab_to_azscene(
        "prefabs/native-component.prefab.ron",
        &source,
        &app_registry,
        &lowerings(),
        |path| Err(format!("unexpected dependency {path}")),
    )
    .expect("process native component prefab");
    let registry = app_registry.read();
    let scene = read_scene_asset_from_reader(compiled.bytes.as_slice(), &registry)
        .expect("decode processed AZSCENE");
    let target = scene.metadata.entities[0]
        .component_targets
        .iter()
        .find(|target| {
            target.native_type_id == <NativeComponentFixture as az_core::AzTypeInfo>::TYPE_ID
        })
        .expect("native component target metadata");

    assert_eq!(target.component_id, component_id);
    assert_eq!(scene.metadata.entities[0].component_targets.len(), 1);
}

#[derive(Reflect, Default)]
struct MultiplierSource {
    authored_amount: f32,
}

#[derive(Component, Reflect, Prefab)]
#[reflect(Component, Prefab)]
#[prefab(
    tag = "Multiplier",
    version = 1,
    template = MultiplierSource,
    construct = build_multiplier
)]
struct Multiplier {
    runtime_amount: f32,
}

// Registered as a Prefab template build callback, so the `Result` return is fixed by
// that signature; dropping it would not compile.
#[allow(clippy::unnecessary_wraps)]
fn build_multiplier(
    template: &MultiplierSource,
    _context: &mut TemplateContext<'_, '_>,
) -> Result<Multiplier, PrefabBuildError> {
    Ok(Multiplier {
        runtime_amount: template.authored_amount * 2.0,
    })
}

// Exact by construction: the override sets `authored_amount` to 7.0 and the template
// doubles it, and multiplying an f32 by 2.0 is exact, so 14.0 is the precise result.
#[allow(clippy::float_cmp)]
// The decoded scene and the materialized world borrow from this type-registry read
// guard for the rest of the test, so it cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
#[test]
fn template_backed_prefab_override_targets_authoring_schema_before_construction() {
    let app_registry = engine_prefab_type_registry();
    {
        let mut registry = app_registry.write();
        registry.register::<MultiplierSource>();
        registry.register::<Multiplier>();
    }
    let registry = app_registry.read();

    let mut base = PrefabDocument::default();
    base.type_versions.insert("Multiplier".to_owned(), 1);
    base.entities.insert(
        alias("fixture"),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([(
                "Multiplier".to_owned(),
                sparse(MultiplierSource {
                    authored_amount: 2.0,
                }),
            )]),
        },
    );
    let base_source = encode(&base, &registry);

    let mut outer = PrefabDocument::default();
    outer.instances.insert(
        InstanceAlias::new("base").unwrap(),
        PrefabInstance {
            source: PrefabAssetPath::new("prefabs/base.prefab.ron").unwrap(),
            parent: None,
            overrides: vec![OverrideOperation {
                target: TypedOverrideTarget::new(
                    Vec::new(),
                    alias("fixture"),
                    Multiplier::type_info().type_path(),
                    ReflectedPath::new(["authored_amount"]).unwrap(),
                )
                .unwrap(),
                action: TypedOverrideAction::Set(sparse(7.0_f32)),
            }],
        },
    );
    let outer_source = encode(&outer, &registry);
    drop(registry);

    let compiled = process_prefab_to_azscene(
        "prefabs/outer.prefab.ron",
        &outer_source,
        &app_registry,
        &lowerings(),
        |path| match path {
            "prefabs/base.prefab.ron" => Ok(base_source.clone()),
            _ => Err(format!("unknown source {path}")),
        },
    )
    .expect("process template-backed Prefab");
    let registry = app_registry.read();
    let loaded = read_scene_asset_from_reader(compiled.bytes.as_slice(), &registry).unwrap();
    let mut world = World::new();
    let instance = loaded.materialize(&mut world, &registry).unwrap();
    assert_eq!(
        world
            .get::<Multiplier>(instance.entities[0])
            .unwrap()
            .runtime_amount,
        14.0
    );
}

/// Builds the base / nested / outer documents this precedence test drives, and
/// hands back their encoded sources plus the registry they were encoded against.
fn nested_override_sources() -> (AppTypeRegistry, String, String, String) {
    let app_registry = engine_prefab_type_registry();
    let registry = app_registry.read();
    let mut base = PrefabDocument::default();
    base.type_versions.insert("Transform".to_owned(), 1);
    base.entities
        .insert(alias("root"), transform_entity(Vec3::X, None));
    base.entities
        .insert(alias("leaf"), transform_entity(Vec3::Y, Some("root")));
    let base_source = encode(&base, &registry);

    let mut nested = PrefabDocument::default();
    nested.instances.insert(
        InstanceAlias::new("base").unwrap(),
        PrefabInstance {
            source: PrefabAssetPath::new("prefabs/base.prefab.ron").unwrap(),
            parent: None,
            overrides: vec![OverrideOperation {
                target: TypedOverrideTarget::new(
                    Vec::new(),
                    alias("root"),
                    Transform::type_info().type_path(),
                    ReflectedPath::new(["position"]).unwrap(),
                )
                .unwrap(),
                action: TypedOverrideAction::Set(sparse(Vec3::new(4.0, 5.0, 6.0))),
            }],
        },
    );
    let nested_source = encode(&nested, &registry);

    let mut outer = PrefabDocument::default();
    outer.type_versions.insert("Transform".to_owned(), 1);
    outer
        .entities
        .insert(alias("mount"), transform_entity(Vec3::ZERO, None));
    outer.instances.insert(
        InstanceAlias::new("nested").unwrap(),
        PrefabInstance {
            source: PrefabAssetPath::new("prefabs/nested.prefab.ron").unwrap(),
            parent: Some(alias("mount")),
            overrides: vec![OverrideOperation {
                target: TypedOverrideTarget::new(
                    vec![InstanceAlias::new("base").unwrap()],
                    alias("root"),
                    Transform::type_info().type_path(),
                    ReflectedPath::new(["position"]).unwrap(),
                )
                .unwrap(),
                action: TypedOverrideAction::Set(sparse(Vec3::new(9.0, 8.0, 7.0))),
            }],
        },
    );
    let outer_source = encode(&outer, &registry);
    drop(registry);
    (app_registry, base_source, nested_source, outer_source)
}

// The decoded scene and the materialized world borrow from this type-registry read
// guard for the rest of the test, so it cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
#[test]
fn nested_prefab_sparse_override_processes_with_inner_to_outer_precedence() {
    let (app_registry, base_source, nested_source, outer_source) = nested_override_sources();

    let first = process_prefab_to_azscene(
        "prefabs/outer.prefab.ron",
        &outer_source,
        &app_registry,
        &lowerings(),
        |path| match path {
            "prefabs/base.prefab.ron" => Ok(base_source.clone()),
            "prefabs/nested.prefab.ron" => Ok(nested_source.clone()),
            _ => Err(format!("unknown source {path}")),
        },
    )
    .unwrap();
    let second = process_prefab_to_azscene(
        "prefabs/outer.prefab.ron",
        &outer_source,
        &app_registry,
        &lowerings(),
        |path| match path {
            "prefabs/base.prefab.ron" => Ok(base_source.clone()),
            "prefabs/nested.prefab.ron" => Ok(nested_source.clone()),
            _ => Err(format!("unknown source {path}")),
        },
    )
    .unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(
        first.source_dependencies,
        vec!["prefabs/base.prefab.ron", "prefabs/nested.prefab.ron"]
    );
    assert_eq!(first.product_path, "prefabs/outer.scn.bin");

    let registry = app_registry.read();
    let loaded = read_scene_asset_from_reader(first.bytes.as_slice(), &registry).unwrap();
    assert_eq!(
        loaded
            .metadata
            .entities
            .iter()
            .map(|entity| entity.source_alias.as_str())
            .collect::<Vec<_>>(),
        vec!["mount", "nested/base/leaf", "nested/base/root"]
    );
    let mut world = World::new();
    let instance = loaded.materialize(&mut world, &registry).unwrap();
    let nested_root = instance.entities[2];
    let nested_leaf = instance.entities[1];
    assert_eq!(
        world.get::<Transform>(nested_root).unwrap().position,
        Vec3::new(9.0, 8.0, 7.0)
    );
    assert_eq!(
        world.get::<ChildOf>(nested_root).unwrap().parent(),
        instance.entities[0]
    );
    assert_eq!(
        world.get::<ChildOf>(nested_leaf).unwrap().parent(),
        nested_root
    );
}

#[test]
fn nested_prefab_cycle_is_rejected_with_the_source_chain() {
    let app_registry = engine_prefab_type_registry();
    let registry = app_registry.read();
    let document = |instance: &str, source: &str| {
        let mut document = PrefabDocument::default();
        document.instances.insert(
            InstanceAlias::new(instance).unwrap(),
            PrefabInstance {
                source: PrefabAssetPath::new(source).unwrap(),
                parent: None,
                overrides: Vec::new(),
            },
        );
        document
    };
    let a = encode(&document("b", "prefabs/b.prefab.ron"), &registry);
    let b = encode(&document("a", "prefabs/a.prefab.ron"), &registry);
    drop(registry);

    let error = process_prefab_to_azscene(
        "prefabs/a.prefab.ron",
        &a,
        &app_registry,
        &lowerings(),
        |path| match path {
            "prefabs/a.prefab.ron" => Ok(a.clone()),
            "prefabs/b.prefab.ron" => Ok(b.clone()),
            _ => Err(format!("unknown source {path}")),
        },
    )
    .unwrap_err();
    assert!(matches!(
        error,
        PrefabAzSceneProcessError::SourceCycle { chain }
            if chain == vec![
                "prefabs/a.prefab.ron".to_owned(),
                "prefabs/b.prefab.ron".to_owned(),
                "prefabs/a.prefab.ron".to_owned(),
            ]
    ));
}

// The decoded scene and the materialized world borrow from this type-registry read
// guard for the rest of the test, so it cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
#[test]
fn authored_transform_mesh_and_light_process_load_and_materialize_typed_state() {
    let intended_transform = Transform {
        position: Vec3::new(4.0, -2.0, 11.5),
        rotation: Quat::from_rotation_y(0.75),
        scale: Vec3::new(2.0, 3.0, 4.0),
    };
    let intended_mesh = Mesh {
        mesh: "models/fixture.azmesh".parse().unwrap(),
        visible: true,
        cast_shadows: false,
        receive_shadows: true,
        lod_bias: -1,
    };
    let intended_light = DirectionalLight {
        color: Vec4::new(0.25, 0.5, 1.0, 1.0),
        illuminance_lux: 42_000.0,
        shadows_enabled: true,
        angular_diameter_degrees: 0.7,
    };
    let mut document = PrefabDocument::default();
    document.type_versions.extend([
        ("DirectionalLight".to_owned(), 1),
        ("Mesh".to_owned(), 2),
        ("Transform".to_owned(), 1),
    ]);
    document.entities.insert(
        alias("fixture"),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([
                ("DirectionalLight".to_owned(), sparse(intended_light)),
                ("Mesh".to_owned(), sparse(intended_mesh.clone())),
                ("Transform".to_owned(), sparse(intended_transform)),
            ]),
        },
    );

    let app_registry = engine_prefab_type_registry();
    let registry = app_registry.read();
    let source = encode(&document, &registry);
    drop(registry);
    let processed = process_prefab_to_azscene(
        "prefabs/fixture.prefab.ron",
        &source,
        &app_registry,
        &lowerings(),
        |_| Err("fixture has no nested sources".to_owned()),
    )
    .unwrap();
    assert!(processed.asset_dependencies.is_empty());

    let registry = app_registry.read();
    let loaded = read_scene_asset_from_reader(processed.bytes.as_slice(), &registry).unwrap();
    let mut world = World::new();
    let instance = loaded.materialize(&mut world, &registry).unwrap();
    assert_eq!(instance.entities.len(), 1);
    let entity = instance.entities[0];
    assert_eq!(world.get::<Transform>(entity), Some(&intended_transform));
    assert_eq!(world.get::<Mesh>(entity), Some(&intended_mesh));
    assert_eq!(world.get::<DirectionalLight>(entity), Some(&intended_light));
}
