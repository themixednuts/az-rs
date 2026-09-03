use std::collections::{BTreeMap, HashMap};

use az_core::{EntityId, component::ComponentId};
use az_scene::{
    AZSCENE_FORMAT_VERSION, AZSCENE_MAGIC, AzSceneAsset, AzSceneCodecError, AzSceneComponentTarget,
    AzSceneEntityMetadata, AzSceneMetadata, AzSceneSourceScopeMetadata, LocalEntityId,
    LocalEntityScopeId, encode_scene_asset, read_scene_asset_from_reader,
    read_scene_metadata_from_reader,
};
use bevy::{
    ecs::{
        component::Component,
        entity::{Entity, EntityMapper, MapEntities},
        hierarchy::{ChildOf, Children},
        reflect::ReflectComponent,
        world::World,
    },
    reflect::{
        Reflect, ReflectDeserialize, ReflectSerialize, TypeRegistry, std_traits::ReflectDefault,
    },
    world_serialization::{DynamicWorld, DynamicWorldBuilder},
};
use serde::{Deserialize, Serialize};

#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
struct NumberComponent {
    value: u32,
}

#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
struct MapComponent {
    values: HashMap<String, u32>,
}

#[derive(Component, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Component)]
#[component(map_entities)]
struct EntityLink {
    target: Entity,
}

#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[reflect(Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
struct LegacyScalarRecord {
    value: u32,
}

// `#[serde(try_from = "u32")]` needs `TryFrom<u32>`, which std's blanket impl
// supplies from this `From` with `Error = Infallible`.
impl From<u32> for LegacyScalarRecord {
    fn from(value: u32) -> Self {
        Self { value }
    }
}

impl From<LegacyScalarRecord> for u32 {
    fn from(value: LegacyScalarRecord) -> Self {
        value.value
    }
}

#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[reflect(Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
enum LegacyIntegerEnum {
    #[default]
    Zero,
    Seven,
}

impl TryFrom<u8> for LegacyIntegerEnum {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Zero),
            7 => Ok(Self::Seven),
            _ => Err("unknown legacy enum value"),
        }
    }
}

impl From<LegacyIntegerEnum> for u8 {
    fn from(value: LegacyIntegerEnum) -> Self {
        match value {
            LegacyIntegerEnum::Zero => 0,
            LegacyIntegerEnum::Seven => 7,
        }
    }
}

#[derive(Component, Reflect, Debug, Clone, PartialEq, Eq)]
#[reflect(Component)]
struct AlternateSerdeComponent {
    record: LegacyScalarRecord,
    mode: LegacyIntegerEnum,
}

#[derive(Reflect, Serialize, Deserialize, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(opaque, Serialize, Deserialize)]
struct ConditionalOpaqueRecord {
    value: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

#[derive(Reflect, Debug, Clone, Default, PartialEq, Eq)]
enum NestedStructuralState {
    #[default]
    Idle,
    Active(ConditionalOpaqueRecord),
}

#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
struct NestedStructuralComponent {
    state: Option<NestedStructuralState>,
    records: BTreeMap<String, ConditionalOpaqueRecord>,
    #[reflect(ignore)]
    runtime_only: u32,
}

#[derive(Reflect, Debug, Clone, Default, PartialEq, Eq)]
enum NestedStructuralOperation {
    #[default]
    Noop,
    Apply {
        record: Option<ConditionalOpaqueRecord>,
        count: u32,
    },
}

#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component, Default)]
struct NestedStructuralOperationsComponent {
    operations: Vec<NestedStructuralOperation>,
}

#[test]
fn named_records_ignore_alternate_serde_vocabularies_on_the_azscene_wire() {
    let mut registry = registry();
    registry.register::<LegacyScalarRecord>();
    registry.register::<LegacyIntegerEnum>();
    registry.register::<AlternateSerdeComponent>();
    let mut source = World::new();
    let entity = source
        .spawn(AlternateSerdeComponent {
            record: LegacyScalarRecord { value: 41 },
            mode: LegacyIntegerEnum::Seven,
        })
        .id();
    let asset = capture(
        &source,
        &registry,
        &[entity],
        vec![AzSceneEntityMetadata {
            source_alias: "alternate-serde".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: None,
            parent: None,
            component_targets: Vec::new(),
        }],
    );

    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let loaded = read_scene_asset_from_reader(bytes.as_slice(), &registry).unwrap();
    let mut destination = World::new();
    let instance = loaded.materialize(&mut destination, &registry).unwrap();
    assert_eq!(
        destination.get::<AlternateSerdeComponent>(instance.entities[0]),
        Some(&AlternateSerdeComponent {
            record: LegacyScalarRecord { value: 41 },
            mode: LegacyIntegerEnum::Seven,
        })
    );
}

#[test]
fn nested_structural_values_decode_from_the_v1_product_codec() {
    let mut registry = registry();
    registry.register::<ConditionalOpaqueRecord>();
    registry.register::<NestedStructuralState>();
    registry.register::<Option<NestedStructuralState>>();
    registry.register::<BTreeMap<String, ConditionalOpaqueRecord>>();
    registry.register::<NestedStructuralComponent>();
    let mut source = World::new();
    let entity = source
        .spawn(NestedStructuralComponent {
            state: Some(NestedStructuralState::Active(ConditionalOpaqueRecord {
                value: 41,
                hint: None,
            })),
            records: BTreeMap::from([(
                "primary".to_owned(),
                ConditionalOpaqueRecord {
                    value: 73,
                    hint: None,
                },
            )]),
            runtime_only: 9,
        })
        .id();
    let asset = capture(
        &source,
        &registry,
        &[entity],
        vec![AzSceneEntityMetadata {
            source_alias: "nested-structural".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: None,
            parent: None,
            component_targets: Vec::new(),
        }],
    );

    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let loaded = read_scene_asset_from_reader(bytes.as_slice(), &registry).unwrap();
    let mut destination = World::new();
    let instance = loaded.materialize(&mut destination, &registry).unwrap();
    assert_eq!(
        destination.get::<NestedStructuralComponent>(instance.entities[0]),
        Some(&NestedStructuralComponent {
            state: Some(NestedStructuralState::Active(ConditionalOpaqueRecord {
                value: 41,
                hint: None,
            })),
            records: BTreeMap::from([(
                "primary".to_owned(),
                ConditionalOpaqueRecord {
                    value: 73,
                    hint: None,
                },
            )]),
            runtime_only: 0,
        })
    );
}

#[test]
fn enum_struct_variants_nested_in_sequences_decode_from_the_v1_product_codec() {
    let mut registry = registry();
    registry.register::<ConditionalOpaqueRecord>();
    registry.register::<Option<ConditionalOpaqueRecord>>();
    registry.register::<NestedStructuralOperation>();
    registry.register::<Vec<NestedStructuralOperation>>();
    registry.register::<NestedStructuralOperationsComponent>();
    let mut source = World::new();
    let entity = source
        .spawn(NestedStructuralOperationsComponent {
            operations: vec![NestedStructuralOperation::Apply {
                record: Some(ConditionalOpaqueRecord {
                    value: 41,
                    hint: None,
                }),
                count: 73,
            }],
        })
        .id();
    let asset = capture(
        &source,
        &registry,
        &[entity],
        vec![AzSceneEntityMetadata {
            source_alias: "enum-struct-variant".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: None,
            parent: None,
            component_targets: Vec::new(),
        }],
    );

    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let loaded = read_scene_asset_from_reader(bytes.as_slice(), &registry).unwrap();
    let mut destination = World::new();
    let instance = loaded.materialize(&mut destination, &registry).unwrap();
    assert_eq!(
        destination.get::<NestedStructuralOperationsComponent>(instance.entities[0]),
        Some(&NestedStructuralOperationsComponent {
            operations: vec![NestedStructuralOperation::Apply {
                record: Some(ConditionalOpaqueRecord {
                    value: 41,
                    hint: None,
                }),
                count: 73,
            }],
        })
    );
}

impl MapEntities for EntityLink {
    fn map_entities<M: EntityMapper>(&mut self, entity_mapper: &mut M) {
        self.target = entity_mapper.get_mapped(self.target);
    }
}

fn registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<Entity>();
    registry.register::<NumberComponent>();
    registry.register::<MapComponent>();
    registry.register::<EntityLink>();
    registry
}

fn capture(
    world: &World,
    registry: &TypeRegistry,
    entities: &[Entity],
    metadata: Vec<AzSceneEntityMetadata>,
) -> AzSceneAsset {
    AzSceneAsset::new_in_entity_order(
        DynamicWorldBuilder::from_world(world, registry)
            .extract_entities(entities.iter().copied())
            .build(),
        entities,
        AzSceneMetadata {
            source_scopes: vec![AzSceneSourceScopeMetadata { parent: None }],
            entities: metadata,
        },
    )
    .unwrap()
}

#[test]
fn repeated_encode_is_byte_identical_and_canonicalizes_maps() {
    let registry = registry();
    let build = |entries: [(&str, u32); 3]| {
        let mut world = World::new();
        let entity = world
            .spawn(MapComponent {
                values: entries
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
            })
            .id();
        capture(
            &world,
            &registry,
            &[entity],
            vec![AzSceneEntityMetadata {
                source_alias: "root".to_owned(),
                source_scope: LocalEntityScopeId::ROOT,
                source_entity_id: None,
                parent: None,
                component_targets: Vec::new(),
            }],
        )
    };
    let first = build([("z", 3), ("a", 1), ("m", 2)]);
    let second = build([("m", 2), ("z", 3), ("a", 1)]);

    let first_bytes = encode_scene_asset(&first, &registry).unwrap().bytes;
    let repeated = encode_scene_asset(&first, &registry).unwrap().bytes;
    let second_bytes = encode_scene_asset(&second, &registry).unwrap().bytes;
    assert_eq!(first_bytes, repeated);
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(&first_bytes[..8], AZSCENE_MAGIC);
    assert_eq!(&first_bytes[8..12], &AZSCENE_FORMAT_VERSION.to_le_bytes());
}

#[test]
fn metadata_reader_preserves_native_component_targets_without_a_type_registry() {
    let registry = registry();
    let mut world = World::new();
    let entity = world.spawn(NumberComponent { value: 7 }).id();
    let native_type_id = uuid::uuid!("6c2fc842-f451-4b45-9d7d-a6ee954df025");
    let metadata = AzSceneMetadata {
        source_scopes: vec![AzSceneSourceScopeMetadata { parent: None }],
        entities: vec![AzSceneEntityMetadata {
            source_alias: "root".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: Some(EntityId::new(4_242)),
            parent: None,
            component_targets: vec![AzSceneComponentTarget {
                native_type_id,
                component_id: ComponentId::new(42),
            }],
        }],
    };
    let asset = capture(&world, &registry, &[entity], metadata.entities.clone());
    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;

    let decoded = read_scene_metadata_from_reader(bytes.as_slice()).unwrap();

    assert_eq!(decoded, metadata);
    assert_eq!(
        decoded.entities[0].component_targets,
        vec![AzSceneComponentTarget {
            native_type_id,
            component_id: ComponentId::new(42),
        }]
    );
}

#[test]
fn source_scope_metadata_roundtrips_and_isolates_sibling_ids() {
    let registry = registry();
    let mut world = World::new();
    let first = world.spawn_empty().id();
    let second = world.spawn_empty().id();
    let shared_source_id = EntityId::new(5_500);
    let metadata = AzSceneMetadata {
        source_scopes: vec![
            AzSceneSourceScopeMetadata { parent: None },
            AzSceneSourceScopeMetadata {
                parent: Some(LocalEntityScopeId::ROOT),
            },
            AzSceneSourceScopeMetadata {
                parent: Some(LocalEntityScopeId::ROOT),
            },
        ],
        entities: vec![
            AzSceneEntityMetadata {
                source_alias: "first/target".to_owned(),
                source_scope: LocalEntityScopeId::new(1),
                source_entity_id: Some(shared_source_id),
                parent: None,
                component_targets: Vec::new(),
            },
            AzSceneEntityMetadata {
                source_alias: "second/target".to_owned(),
                source_scope: LocalEntityScopeId::new(2),
                source_entity_id: Some(shared_source_id),
                parent: None,
                component_targets: Vec::new(),
            },
        ],
    };
    let asset = AzSceneAsset::new_in_entity_order(
        DynamicWorldBuilder::from_world(&world, &registry)
            .extract_entities([first, second].into_iter())
            .build(),
        &[first, second],
        metadata.clone(),
    )
    .unwrap();

    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;

    assert_eq!(
        read_scene_metadata_from_reader(bytes.as_slice()).unwrap(),
        metadata
    );
}

#[test]
fn codec_rejects_duplicate_source_ids_within_one_scope() {
    let registry = registry();
    let mut world = World::new();
    let first = world.spawn_empty().id();
    let second = world.spawn_empty().id();
    let shared_source_id = EntityId::new(5_501);
    let metadata = AzSceneMetadata {
        source_scopes: vec![AzSceneSourceScopeMetadata { parent: None }],
        entities: vec![
            AzSceneEntityMetadata {
                source_alias: "first".to_owned(),
                source_scope: LocalEntityScopeId::ROOT,
                source_entity_id: Some(shared_source_id),
                parent: None,
                component_targets: Vec::new(),
            },
            AzSceneEntityMetadata {
                source_alias: "second".to_owned(),
                source_scope: LocalEntityScopeId::ROOT,
                source_entity_id: Some(shared_source_id),
                parent: None,
                component_targets: Vec::new(),
            },
        ],
    };
    let asset = AzSceneAsset::new_in_entity_order(
        DynamicWorldBuilder::from_world(&world, &registry)
            .extract_entities([first, second].into_iter())
            .build(),
        &[first, second],
        metadata,
    )
    .unwrap();

    assert!(matches!(
        encode_scene_asset(&asset, &registry),
        Err(AzSceneCodecError::DuplicateSourceEntityId {
            source_entity_id,
            ..
        }) if source_entity_id == shared_source_id
    ));
}

#[test]
fn reader_rejects_malformed_source_scope_table() {
    let registry = registry();
    let asset = AzSceneAsset::new(DynamicWorld::default(), AzSceneMetadata::default());
    let mut bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let root_parent_offset = 8 + 4 + 5 * 4 + 4;
    bytes[root_parent_offset..root_parent_offset + 4].copy_from_slice(&0_u32.to_le_bytes());

    assert!(matches!(
        read_scene_metadata_from_reader(bytes.as_slice()),
        Err(AzSceneCodecError::InvalidSourceScopeParent {
            source_scope: 0,
            parent: 0,
        })
    ));
}

#[test]
fn reader_rejects_unknown_versions_and_malformed_limits() {
    for version in [0, 2, u32::MAX] {
        let mut bytes = AZSCENE_MAGIC.to_vec();
        bytes.extend_from_slice(&version.to_le_bytes());
        let error = read_scene_asset_from_reader(bytes.as_slice(), &registry()).unwrap_err();
        assert!(matches!(
            error,
            AzSceneCodecError::UnsupportedVersion { version: found, .. } if found == version
        ));
    }

    let mut bytes = AZSCENE_MAGIC.to_vec();
    bytes.extend_from_slice(&AZSCENE_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&(64_u32 * 1024 + 1).to_le_bytes());
    let error = read_scene_asset_from_reader(bytes.as_slice(), &registry()).unwrap_err();
    assert!(matches!(
        error,
        AzSceneCodecError::CountTooLarge { kind: "types", .. }
    ));
}

#[test]
fn reader_rejects_unregistered_type_before_payload_allocation() {
    let type_path = "missing::UnregisteredComponent";
    let mut bytes = AZSCENE_MAGIC.to_vec();
    bytes.extend_from_slice(&AZSCENE_FORMAT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&u32::try_from(type_path.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(type_path.as_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&u32::MAX.to_le_bytes());

    let error =
        read_scene_asset_from_reader(bytes.as_slice(), &TypeRegistry::default()).unwrap_err();
    assert!(matches!(
        error,
        AzSceneCodecError::UnregisteredType { type_path: found } if found == type_path
    ));
}

#[test]
fn entity_references_hierarchy_instances_remove_and_reload_are_isolated() {
    let registry = registry();
    let mut source = World::new();
    let root = source.spawn(NumberComponent { value: 7 }).id();
    let child = source.spawn(EntityLink { target: root }).id();
    let asset = capture(
        &source,
        &registry,
        &[root, child],
        vec![
            AzSceneEntityMetadata {
                source_alias: "root".to_owned(),
                source_scope: LocalEntityScopeId::ROOT,
                source_entity_id: None,
                parent: None,
                component_targets: Vec::new(),
            },
            AzSceneEntityMetadata {
                source_alias: "child".to_owned(),
                source_scope: LocalEntityScopeId::ROOT,
                source_entity_id: None,
                parent: Some(LocalEntityId::new(0)),
                component_targets: Vec::new(),
            },
        ],
    );
    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let loaded = read_scene_asset_from_reader(bytes.as_slice(), &registry).unwrap();

    let mut destination = World::new();
    let mut first = loaded.materialize(&mut destination, &registry).unwrap();
    let mut second = loaded.materialize(&mut destination, &registry).unwrap();
    assert_ne!(first.entities, second.entities);
    assert_eq!(
        destination
            .get::<EntityLink>(first.entities[1])
            .unwrap()
            .target,
        first.entities[0]
    );
    assert_eq!(
        destination
            .get::<EntityLink>(second.entities[1])
            .unwrap()
            .target,
        second.entities[0]
    );
    assert_eq!(
        destination
            .get::<ChildOf>(first.entities[1])
            .unwrap()
            .parent(),
        first.entities[0]
    );
    assert_eq!(
        destination
            .get::<Children>(first.entities[0])
            .unwrap()
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![first.entities[1]]
    );

    assert_eq!(first.remove(&mut destination), 2);
    assert_eq!(first.remove(&mut destination), 0);
    assert!(destination.get_entity(second.entities[0]).is_ok());
    assert!(destination.get_entity(second.entities[1]).is_ok());

    let mut reloaded = loaded.materialize(&mut destination, &registry).unwrap();
    assert_eq!(
        destination
            .get::<NumberComponent>(reloaded.entities[0])
            .unwrap()
            .value,
        7
    );
    assert_eq!(second.remove(&mut destination), 2);
    assert_eq!(reloaded.remove(&mut destination), 2);
}

#[test]
fn client_and_server_registries_load_the_same_typed_state() {
    let source_registry = registry();
    let mut source = World::new();
    let entity = source.spawn(NumberComponent { value: 42 }).id();
    let asset = capture(
        &source,
        &source_registry,
        &[entity],
        vec![AzSceneEntityMetadata {
            source_alias: "root".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: None,
            parent: None,
            component_targets: Vec::new(),
        }],
    );
    let bytes = encode_scene_asset(&asset, &source_registry).unwrap().bytes;

    for role_registry in [registry(), registry()] {
        let loaded = read_scene_asset_from_reader(bytes.as_slice(), &role_registry).unwrap();
        let mut world = World::new();
        let instance = loaded.materialize(&mut world, &role_registry).unwrap();
        assert_eq!(
            world.get::<NumberComponent>(instance.entities[0]),
            Some(&NumberComponent { value: 42 })
        );
    }
}

#[test]
fn materialization_rejects_missing_registration_and_rolls_back_allocations() {
    let registry = registry();
    let mut source = World::new();
    let entity = source.spawn(NumberComponent { value: 99 }).id();
    let asset = capture(
        &source,
        &registry,
        &[entity],
        vec![AzSceneEntityMetadata {
            source_alias: "missing-registration".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: None,
            parent: None,
            component_targets: Vec::new(),
        }],
    );
    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let loaded = read_scene_asset_from_reader(bytes.as_slice(), &registry).unwrap();
    let mut incomplete = TypeRegistry::default();
    incomplete.register::<Entity>();
    let mut destination = World::new();
    let entity_count_before = destination.iter_entities().count();
    assert!(loaded.materialize(&mut destination, &incomplete).is_err());
    assert_eq!(destination.iter_entities().count(), entity_count_before);
}

#[test]
fn type_and_component_records_are_stably_ordered() {
    let mut registry = registry();
    registry.register::<BTreeMap<String, u32>>();
    let mut world = World::new();
    let entity = world
        .spawn((
            MapComponent {
                values: HashMap::from([("b".to_owned(), 2), ("a".to_owned(), 1)]),
            },
            NumberComponent { value: 9 },
        ))
        .id();
    let asset = capture(
        &world,
        &registry,
        &[entity],
        vec![AzSceneEntityMetadata {
            source_alias: "entity".to_owned(),
            source_scope: LocalEntityScopeId::ROOT,
            source_entity_id: None,
            parent: None,
            component_targets: Vec::new(),
        }],
    );
    let bytes = encode_scene_asset(&asset, &registry).unwrap().bytes;
    let loaded = read_scene_asset_from_reader(bytes.as_slice(), &registry).unwrap();
    let paths = loaded.dynamic_world.entities[0]
        .components
        .iter()
        .map(|component| component.get_represented_type_info().unwrap().type_path())
        .collect::<Vec<_>>();
    assert!(paths.windows(2).all(|pair| pair[0] < pair[1]));
}
