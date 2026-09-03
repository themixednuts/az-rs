use std::{any::TypeId, collections::BTreeMap};

use az_core::EntityId;
use az_prefab::{
    EntityAlias, ErasedPrefabValue, InstanceAlias, OverrideOperation, Prefab, PrefabAssetPath,
    PrefabBuildError, PrefabCodec, PrefabCodecError, PrefabConstruction, PrefabDocument,
    PrefabEntity, PrefabInstance, PrefabMigrationError, PrefabMigrationStep, PrefabProductPolicy,
    PrefabRegistry, PrefabTagAlias, PrefabTypeData, ReflectPrefab, ReflectedPath, SparseValue,
    TypedOverrideAction, TypedOverrideTarget, TypedPrefabSemantics, TypedPrefabSemanticsError,
    type_data::{construct_reflected, insert_reflected_component},
};
use bevy_ecs::{
    component::Component,
    reflect::{AppTypeRegistry, ReflectComponent},
    template::{SceneEntityReferences, TemplateContext},
    world::World,
};
use bevy_reflect::{
    PartialReflect, Reflect, ReflectDeserialize, ReflectSerialize, TypeRegistry, Typed,
    std_traits::ReflectDefault, structs::DynamicStruct,
};
use serde::{Deserialize, Serialize};

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "CodecFixture", version = 1)]
struct CodecFixture {
    amount: f32,
    enabled: bool,
}

#[derive(Reflect, Default)]
struct TemplateSourceFixture {
    authored_amount: f32,
    authored_enabled: bool,
}

#[derive(Component, Reflect, Prefab)]
#[reflect(Component, Prefab)]
#[prefab(
    tag = "TemplateProductFixture",
    version = 1,
    template = TemplateSourceFixture,
    construct = build_template_product_fixture
)]
struct TemplateProductFixture {
    runtime_amount: f32,
    runtime_enabled: bool,
}

// The signature is fixed by the `#[prefab(construct = ...)]` callback contract,
// so the infallible body still has to return a `Result`.
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the #[prefab(construct = ...)] callback signature"
)]
fn build_template_product_fixture(
    template: &TemplateSourceFixture,
    _context: &mut TemplateContext<'_, '_>,
) -> Result<TemplateProductFixture, PrefabBuildError> {
    Ok(TemplateProductFixture {
        runtime_amount: template.authored_amount * 2.0,
        runtime_enabled: template.authored_enabled,
    })
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "VersionZeroFixture", version = 0)]
struct VersionZeroFixture;

#[derive(Reflect, Default)]
enum EnumMode {
    #[default]
    Disabled,
    Enabled {
        threshold: f32,
        active: bool,
    },
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "EnumFixture", version = 1)]
struct EnumFixture {
    mode: EnumMode,
}

#[derive(Reflect, Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
#[repr(i32)]
#[serde(try_from = "i32", into = "i32")]
enum LegacyIntegerEnum {
    #[default]
    HeadSlot = 0,
    BagSlot3 = 32,
}

impl From<LegacyIntegerEnum> for i32 {
    fn from(value: LegacyIntegerEnum) -> Self {
        value as Self
    }
}

impl TryFrom<i32> for LegacyIntegerEnum {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::HeadSlot),
            32 => Ok(Self::BagSlot3),
            other => Err(other),
        }
    }
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "LegacyIntegerEnumFixture", version = 1)]
struct LegacyIntegerEnumFixture {
    slot: LegacyIntegerEnum,
}

#[derive(Reflect, Default)]
struct NestedFixture {
    amount: f32,
    enabled: bool,
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "OptionFixture", version = 1)]
struct OptionFixture {
    nested: Option<NestedFixture>,
}

#[derive(Component, Reflect, Default, Prefab, Serialize, Deserialize)]
#[reflect(Component, Default, Prefab, Serialize, Deserialize)]
#[prefab(tag = "SerdeComponentFixture", version = 1)]
struct SerdeComponentFixture {
    amount: f32,
    enabled: bool,
}

// Legacy-named serde vocabulary on a Prefab component: the serde derive keys
// (`"Preload Name"`, `"Alpha Scale"`) are illegal RON identifiers. The codec
// must ignore the serde impl for Prefab-tagged types (encode and decode gate on
// `PrefabTypeData`) and route structurally with Rust-ident keys.
#[derive(Component, Reflect, Default, Prefab, Serialize, Deserialize)]
#[reflect(Component, Default, Prefab, Serialize, Deserialize)]
#[prefab(tag = "LegacyRenameFixture", version = 1)]
struct LegacyRenameFixture {
    #[serde(rename = "Preload Name", default)]
    preload_name: String,
    #[serde(rename = "Alpha Scale", default)]
    alpha_scale: f32,
}

// Stand-in for a glam-style serde leaf: `#[reflect(Serialize, Deserialize)]`
// without `Prefab` marks the serde impl as the canonical wire, so the codec
// must decline and let the serde impl speak (here distinguishable from the
// structural wire by the `X`/`Y` keys).
#[derive(Reflect, Debug, PartialEq, Default, Clone, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
struct GlamLikeLeaf {
    #[serde(rename = "X", default)]
    x: f32,
    #[serde(rename = "Y", default)]
    y: f32,
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "LeafHostFixture", version = 1)]
struct LeafHostFixture {
    leaf: GlamLikeLeaf,
}

// Non-Prefab nested config without serde reflect type data: the serde derive
// (with its legacy-renamed keys) exists solely for the legacy ObjectStream
// import path; the codec's canonical wire is structural Rust-ident keys.
#[derive(Reflect, Default, Serialize, Deserialize)]
struct StructuralConfig {
    #[serde(rename = "Config Value", default)]
    config_value: f32,
}

#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "ConfigHostFixture", version = 1)]
struct ConfigHostFixture {
    configuration: StructuralConfig,
}

#[derive(Component, Reflect, Default)]
#[reflect(Component, Default)]
struct MeshFixture {
    mesh: String,
    visible: bool,
    cast_shadows: bool,
    receive_shadows: bool,
    lod_bias: i32,
}

// Exercises reflect-opaque identity/CRC leaves in `HashSet` element and
// `HashMap` key position. With the default derived tuple-struct reflection on
// `EntityId`/`Crc32` (no `#[reflect(opaque)]` / `#[reflect(Hash, PartialEq)]`),
// encoding panics in `to_dynamic()` while building a `DynamicSet`, and decoding
// panics rebuilding `DynamicSet`/`DynamicMap` — both in `bevy_reflect` hashing.
#[derive(Component, Reflect, Default, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "SetHashFixture", version = 1)]
struct SetHashFixture {
    entities: std::collections::HashSet<az_core::EntityId>,
    crcs: std::collections::HashSet<az_core::crc::Crc32>,
    weights: std::collections::HashMap<az_core::crc::Crc32, i32>,
}

fn codec_registry() -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<CodecFixture>();
    registry.register::<TemplateSourceFixture>();
    registry.register::<TemplateProductFixture>();
    registry.register::<EnumFixture>();
    registry.register::<LegacyIntegerEnumFixture>();
    registry.register::<OptionFixture>();
    registry.register::<SerdeComponentFixture>();
    registry.register::<VersionZeroFixture>();
    registry.register::<LegacyRenameFixture>();
    registry.register::<GlamLikeLeaf>();
    registry.register::<LeafHostFixture>();
    registry.register::<StructuralConfig>();
    registry.register::<ConfigHostFixture>();
    registry
}

#[test]
fn template_backed_prefab_decodes_and_encodes_its_authoring_schema() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"TemplateProductFixture": 1},
        entities: {
            "fixture": (components: {
                "TemplateProductFixture": (authored_amount: 3.5, authored_enabled: true),
            }),
        },
        instances: {},
    )"#;

    let document = codec.decode(source).expect("decode authoring template");
    let value = &document.entities[&EntityAlias::new("fixture").unwrap()].components["TemplateProductFixture"];
    assert_eq!(
        value.type_info().type_id(),
        TemplateSourceFixture::type_info().type_id()
    );
    TypedPrefabSemantics::validate_local(&document, &registry).expect("validate template source");

    let encoded = codec.encode(&document).expect("encode authoring template");
    assert!(encoded.contains("authored_amount"));
    assert!(encoded.contains("authored_enabled"));
    assert!(!encoded.contains("runtime_amount"));
}

#[test]
fn template_backed_prefab_requires_its_authoring_type_registration() {
    let mut registry = TypeRegistry::default();
    registry.register::<TemplateProductFixture>();

    assert!(matches!(
        PrefabCodec::new(&registry),
        Err(PrefabCodecError::Migration(
            PrefabMigrationError::UnregisteredTemplateType { .. }
        ))
    ));
}

#[test]
fn codec_preserves_a_registered_version_zero_component() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"VersionZeroFixture": 0},
        entities: {"marker": (components: {"VersionZeroFixture": ()})},
        instances: {},
    )"#;

    let document = codec.decode(source).expect("decode version-zero component");
    assert_eq!(document.type_versions["VersionZeroFixture"], 0);
    let encoded = codec
        .encode(&document)
        .expect("encode version-zero component");
    assert!(encoded.contains("\"VersionZeroFixture\": 0"));
}

fn mesh_registry(
    migration: fn(ErasedPrefabValue) -> Result<ErasedPrefabValue, PrefabBuildError>,
) -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<MeshFixture>();
    let registration = registry
        .get_mut(TypeId::of::<MeshFixture>())
        .expect("Mesh fixture registration");
    registration.insert(PrefabTypeData {
        tag: "Mesh",
        source_version: 1 + 1,
        aliases: &[PrefabTagAlias {
            tag: "MeshComponent",
            source_version: 1,
        }],
        migrations: Box::leak(Box::new([PrefabMigrationStep {
            from_version: 1,
            to_version: 1 + 1,
            migrate: migration,
        }])),
        construction: PrefabConstruction::ReflectDefaultOrFromWorld,
        product_policy: PrefabProductPolicy::Runtime,
        construct: construct_reflected,
        insert: insert_reflected_component,
    });
    registry
}

fn migrate_mesh_v1_to_v2(
    mut value: ErasedPrefabValue,
) -> Result<ErasedPrefabValue, PrefabBuildError> {
    let mut sparse = value
        .value
        .reflect_ref()
        .as_struct()
        .map(bevy_reflect::structs::Struct::to_dynamic_struct)
        .map_err(|_| PrefabBuildError::ApplyFailed {
            type_path: MeshFixture::type_info().type_path(),
            message: "expected sparse Mesh struct".to_owned(),
        })?;
    sparse.insert("visible", true);
    sparse.insert("cast_shadows", true);
    sparse.insert("receive_shadows", true);
    sparse.insert("lod_bias", 0_i32);
    value.value = Box::new(sparse);
    Ok(value)
}

fn migrate_with_unknown_field(
    value: ErasedPrefabValue,
) -> Result<ErasedPrefabValue, PrefabBuildError> {
    let mut value = migrate_mesh_v1_to_v2(value)?;
    let mut sparse = value
        .value
        .reflect_ref()
        .as_struct()
        .map(bevy_reflect::structs::Struct::to_dynamic_struct)
        .map_err(|_| PrefabBuildError::ApplyFailed {
            type_path: MeshFixture::type_info().type_path(),
            message: "expected sparse Mesh struct".to_owned(),
        })?;
    sparse.insert("removed_legacy_field", 7_i32);
    value.value = Box::new(sparse);
    Ok(value)
}

#[test]
fn codec_round_trip_preserves_optional_authored_entity_id() {
    let registry = TypeRegistry::default();
    let codec = PrefabCodec::new(&registry).unwrap();
    let source_entity_id = EntityId::new(6_123_109_558_530_746_987);
    let mut document = PrefabDocument::default();
    let identified = PrefabEntity {
        entity_id: Some(source_entity_id),
        ..PrefabEntity::default()
    };
    document.entities.insert(entity("sp01"), identified);
    document
        .entities
        .insert(entity("without_source_id"), PrefabEntity::default());

    let encoded = codec.encode(&document).expect("encode authored identity");
    let decoded = codec.decode(&encoded).expect("decode authored identity");

    assert_eq!(
        decoded.entities[&entity("sp01")].entity_id,
        Some(source_entity_id)
    );
    assert_eq!(
        decoded.entities[&entity("without_source_id")].entity_id,
        None
    );
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "the point of this assertion is that the omitted field kept its exact default; an epsilon would let a nearby value pass"
)]
fn sparse_ron_round_trip_preserves_omitted_vs_explicit_default() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"CodecFixture": 1},
        entities: {
            "explicit": (components: {"CodecFixture": (amount: 0.0)}),
            "omitted": (components: {"CodecFixture": ()}),
            "late": (components: {"CodecFixture": (enabled: false)}),
        },
        instances: {},
    )"#;

    let document = codec.decode(source).expect("decode sparse source");
    let explicit = document.entities[&entity("explicit")].components["CodecFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("dynamic struct");
    let omitted = document.entities[&entity("omitted")].components["CodecFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("dynamic struct");
    let late = document.entities[&entity("late")].components["CodecFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("dynamic struct");
    assert_eq!(
        explicit
            .field("amount")
            .and_then(|value| value.try_downcast_ref::<f32>()),
        Some(&0.0)
    );
    assert!(explicit.field("enabled").is_none());
    assert!(omitted.field("amount").is_none());
    assert!(omitted.field("enabled").is_none());
    assert!(late.field("amount").is_none());
    assert_eq!(
        late.field("enabled")
            .and_then(|value| value.try_downcast_ref::<bool>()),
        Some(&false)
    );

    let encoded = codec.encode(&document).expect("encode sparse source");
    let round_trip = codec.decode(&encoded).expect("round trip");
    let explicit = round_trip.entities[&entity("explicit")].components["CodecFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("dynamic struct");
    let omitted = round_trip.entities[&entity("omitted")].components["CodecFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("dynamic struct");
    let late = round_trip.entities[&entity("late")].components["CodecFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("dynamic struct");
    assert!(explicit.field("amount").is_some());
    assert!(omitted.field("amount").is_none());
    assert!(late.field("amount").is_none());
    assert!(late.field("enabled").is_some());
    assert!(encoded.contains("amount"));

    let registration = registry
        .get(TypeId::of::<CodecFixture>())
        .expect("Codec fixture registration");
    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity_id = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity_id);
        construct_reflected(
            registration,
            round_trip.entities[&entity("explicit")].components["CodecFixture"].value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("materialize sparse source")
    };
    let concrete = built
        .value
        .try_downcast_ref::<CodecFixture>()
        .expect("typed component");
    assert_eq!(concrete.amount, 0.0);
    assert!(!concrete.enabled);
}

#[test]
fn sparse_enum_struct_variant_round_trip_preserves_field_presence() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"EnumFixture": 1},
        entities: {
            "enum": (components: {
                "EnumFixture": (mode: Enabled(threshold: 0.0)),
            }),
        },
        instances: {},
    )"#;

    let first = codec.decode(source).expect("decode sparse enum");
    let encoded = codec.encode(&first).expect("encode sparse enum");
    let second = codec.decode(&encoded).expect("round trip sparse enum");
    let component = second.entities[&entity("enum")].components["EnumFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("fixture struct");
    let mode = component
        .field("mode")
        .expect("explicit enum field")
        .reflect_ref()
        .as_enum()
        .expect("dynamic enum");
    assert_eq!(mode.variant_name(), "Enabled");
    assert!(mode.field("threshold").is_some());
    assert!(mode.field("active").is_none());
    assert!(encoded.contains("threshold"));
    assert!(!encoded.contains("active"));
}

#[test]
fn serde_backed_integer_enum_uses_reflected_variant_in_prefab_ron() {
    let registry = codec_registry();
    assert!(
        registry
            .get(TypeId::of::<LegacyIntegerEnum>())
            .expect("legacy integer enum registration")
            .contains::<ReflectDeserialize>(),
        "fixture must exercise the serde-backed enum path"
    );
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"LegacyIntegerEnumFixture": 1},
        entities: {
            "enum": (components: {
                "LegacyIntegerEnumFixture": (slot: BagSlot3),
            }),
        },
        instances: {},
    )"#;

    let document = codec
        .decode(source)
        .expect("decode reflected enum variant instead of integer serde wire");
    let encoded = codec
        .encode(&document)
        .expect("encode reflected enum variant");
    assert!(
        encoded.contains("BagSlot3"),
        "encoded Prefab RON:\n{encoded}"
    );
    assert!(!encoded.contains("32"), "encoded Prefab RON:\n{encoded}");

    let component = document.entities[&entity("enum")].components["LegacyIntegerEnumFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("fixture struct");
    let slot = component
        .field("slot")
        .expect("slot field")
        .reflect_ref()
        .as_enum()
        .expect("dynamic enum");
    assert_eq!(slot.variant_name(), "BagSlot3");
}

#[test]
fn sparse_option_round_trip_keeps_inner_struct_sparse() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"OptionFixture": 1},
        entities: {
            "option": (components: {
                "OptionFixture": (nested: Some((amount: 0.0))),
            }),
        },
        instances: {},
    )"#;

    let first = codec.decode(source).expect("decode sparse Option");
    let encoded = codec.encode(&first).expect("encode sparse Option");
    let second = codec.decode(&encoded).expect("round trip sparse Option");
    let component = second.entities[&entity("option")].components["OptionFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("fixture struct");
    let option = component
        .field("nested")
        .expect("explicit Option field")
        .reflect_ref()
        .as_enum()
        .expect("dynamic Option");
    assert_eq!(option.variant_name(), "Some");
    let nested = option
        .field_at(0)
        .expect("Some payload")
        .reflect_ref()
        .as_struct()
        .expect("sparse nested struct");
    assert!(nested.field("amount").is_some());
    assert!(nested.field("enabled").is_none());
}

#[test]
fn sparse_option_round_trip_distinguishes_explicit_none_from_absence() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"OptionFixture": 1},
        entities: {
            "explicit-none": (components: {
                "OptionFixture": (nested: None),
            }),
            "absent": (components: {
                "OptionFixture": (),
            }),
        },
        instances: {},
    )"#;

    let first = codec.decode(source).expect("decode sparse Options");
    let encoded = codec.encode(&first).expect("encode sparse Options");
    let second = codec.decode(&encoded).expect("round trip sparse Options");

    let explicit = second.entities[&entity("explicit-none")].components["OptionFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("explicit fixture struct");
    let option = explicit
        .field("nested")
        .expect("explicit None remains represented")
        .reflect_ref()
        .as_enum()
        .expect("dynamic Option");
    assert_eq!(option.variant_name(), "None");

    let absent = second.entities[&entity("absent")].components["OptionFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("absent fixture struct");
    assert!(absent.field("nested").is_none());
}

#[test]
fn serde_backed_prefab_root_stays_sparse() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"SerdeComponentFixture": 1},
        entities: {
            "serde": (components: {
                "SerdeComponentFixture": (amount: 4.0),
            }),
        },
        instances: {},
    )"#;

    let document = codec.decode(source).expect("decode serde-backed Prefab");
    let component = document.entities[&entity("serde")].components["SerdeComponentFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse serde-backed struct");
    assert!(component.field("amount").is_some());
    assert!(component.field("enabled").is_none());

    let encoded = codec.encode(&document).expect("encode sparse Prefab");
    assert!(encoded.contains("amount"));
    assert!(!encoded.contains("enabled"));
}

fn concrete_component_document(tag: &str, component: impl PartialReflect) -> PrefabDocument {
    let mut document = PrefabDocument::default();
    document.type_versions.insert(tag.to_owned(), 1);
    document.entities.insert(
        entity("host"),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([(
                tag.to_owned(),
                SparseValue::try_new(Box::new(component)).unwrap(),
            )]),
        },
    );
    document
}

#[test]
#[expect(
    clippy::float_cmp,
    reason = "this asserts the exact authored value survived the rename round trip; an epsilon would let a corrupted value pass"
)]
fn concrete_prefab_with_legacy_serde_renames_encodes_with_rust_ident_keys() {
    // Regression for `Invalid identifier "Preload Name"`: encoding a CONCRETE
    // Prefab-derived component whose serde impl uses space-renamed keys must
    // route structurally (the serde impl is for the legacy ObjectStream import
    // only), not through `ReflectSerialize`.
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let document = concrete_component_document(
        "LegacyRenameFixture",
        LegacyRenameFixture {
            preload_name: "loadout".to_owned(),
            alpha_scale: 0.5,
        },
    );

    let encoded = codec
        .encode(&document)
        .expect("encode legacy-renamed Prefab component");
    assert!(encoded.contains("preload_name"));
    assert!(encoded.contains("alpha_scale"));
    assert!(!encoded.contains("Preload Name"));
    assert!(!encoded.contains("Alpha Scale"));

    let round_trip = codec.decode(&encoded).expect("round trip");
    let registration = registry
        .get(TypeId::of::<LegacyRenameFixture>())
        .expect("LegacyRenameFixture registration");
    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity_id = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity_id);
        construct_reflected(
            registration,
            round_trip.entities[&entity("host")].components["LegacyRenameFixture"].value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("materialize legacy-renamed component")
    };
    let concrete = built
        .value
        .try_downcast_ref::<LegacyRenameFixture>()
        .expect("typed component");
    assert_eq!(concrete.preload_name, "loadout");
    assert_eq!(concrete.alpha_scale, 0.5);
}

#[test]
fn serde_leaf_without_prefab_data_keeps_its_serde_wire() {
    // A `#[reflect(Serialize, Deserialize)]` leaf without `Prefab` (the
    // glam-style case) is canonical serde wire: the codec must decline and the
    // encoded text must carry the serde keys (`X`/`Y`), not the structural
    // Rust idents.
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let document = concrete_component_document(
        "LeafHostFixture",
        LeafHostFixture {
            leaf: GlamLikeLeaf { x: 1.0, y: 2.0 },
        },
    );

    let encoded = codec.encode(&document).expect("encode serde leaf");
    assert!(encoded.contains("X:"));
    assert!(encoded.contains("Y:"));
    assert!(!encoded.contains("x:"));
    assert!(!encoded.contains("y:"));

    let round_trip = codec.decode(&encoded).expect("round trip serde leaf");
    let component = round_trip.entities[&entity("host")].components["LeafHostFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse host struct");
    let leaf = component
        .field("leaf")
        .expect("leaf field")
        .try_downcast_ref::<GlamLikeLeaf>()
        .expect("concrete serde leaf");
    assert_eq!(leaf, &GlamLikeLeaf { x: 1.0, y: 2.0 });

    // Determinism: encoding the decoded document reproduces the same text.
    let second = codec.encode(&round_trip).expect("second encode");
    assert_eq!(encoded, second);
}

#[test]
fn dynamic_serde_leaf_uses_the_same_registered_wire_as_its_concrete_value() {
    // Imported source documents are dynamic reflected structs. That storage
    // representation must not change a serde-backed leaf's canonical wire.
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let dynamic = concrete_component_document(
        "LeafHostFixture",
        bevy_reflect::structs::Struct::to_dynamic_struct(&LeafHostFixture {
            leaf: GlamLikeLeaf { x: 1.0, y: 2.0 },
        }),
    );
    assert!(
        dynamic.entities[&entity("host")].components["LeafHostFixture"]
            .value()
            .is_dynamic()
    );

    let encoded = codec.encode(&dynamic).expect("encode dynamic serde leaf");
    assert!(encoded.contains("X:"));
    assert!(encoded.contains("Y:"));
    assert!(!encoded.contains("x:"));
    assert!(!encoded.contains("y:"));

    let round_trip = codec
        .decode(&encoded)
        .expect("round trip dynamic serde leaf");
    let component = round_trip.entities[&entity("host")].components["LeafHostFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse host struct");
    assert_eq!(
        component
            .field("leaf")
            .expect("leaf field")
            .try_downcast_ref::<GlamLikeLeaf>(),
        Some(&GlamLikeLeaf { x: 1.0, y: 2.0 })
    );
}

#[test]
fn non_prefab_config_without_serde_type_data_encodes_structurally() {
    // A nested non-Prefab config that does NOT register serde reflect type
    // data must encode structurally with Rust-ident keys even though its serde
    // derive uses a space-renamed key for the legacy ObjectStream import.
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let document = concrete_component_document(
        "ConfigHostFixture",
        ConfigHostFixture {
            configuration: StructuralConfig { config_value: 3.5 },
        },
    );

    let encoded = codec.encode(&document).expect("encode structural config");
    assert!(encoded.contains("configuration"));
    assert!(encoded.contains("config_value"));
    assert!(!encoded.contains("Config Value"));

    let round_trip = codec
        .decode(&encoded)
        .expect("round trip structural config");
    let component = round_trip.entities[&entity("host")].components["ConfigHostFixture"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("sparse host struct");
    let configuration = component
        .field("configuration")
        .expect("configuration field")
        .reflect_ref()
        .as_struct()
        .expect("structural nested config");
    assert_eq!(
        configuration
            .field("config_value")
            .and_then(|value| value.try_downcast_ref::<f32>()),
        Some(&3.5)
    );
}

#[test]
fn set_and_map_leaf_hashing_round_trips_without_panicking() {
    // Regression for the `DynamicSet`/`DynamicMap` hashing panic: reflect-opaque
    // identity/CRC leaves must hash through their concrete `Hash`/`Eq` impls.
    // Encode covers `validate_canonical_document` (the former `to_dynamic()`
    // deep-copy vector) and serialization; decode covers rebuilding the dynamic
    // set/map. This test panics (rather than fails) if `EntityId`/`Crc32` lose
    // `#[reflect(opaque)]` + `#[reflect(Hash, PartialEq)]`.
    let mut registry = TypeRegistry::default();
    registry.register::<SetHashFixture>();
    registry.register::<az_core::EntityId>();
    registry.register::<az_core::crc::Crc32>();
    let codec = PrefabCodec::new(&registry).expect("codec");

    let entities: std::collections::HashSet<az_core::EntityId> =
        [az_core::EntityId::new(7), az_core::EntityId::new(42)]
            .into_iter()
            .collect();
    let crcs: std::collections::HashSet<az_core::crc::Crc32> = [
        az_core::crc::Crc32::from_u32(0xAABB_CCDD),
        az_core::crc::Crc32::from_u32(0x1234_5678),
    ]
    .into_iter()
    .collect();
    let weights: std::collections::HashMap<az_core::crc::Crc32, i32> = [
        (az_core::crc::Crc32::from_u32(1), 10),
        (az_core::crc::Crc32::from_u32(2), 20),
    ]
    .into_iter()
    .collect();

    let fixture = SetHashFixture {
        entities: entities.clone(),
        crcs: crcs.clone(),
        weights: weights.clone(),
    };

    let mut document = PrefabDocument::default();
    document
        .type_versions
        .insert("SetHashFixture".to_owned(), 1);
    document.entities.insert(
        entity("host"),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([(
                "SetHashFixture".to_owned(),
                SparseValue::try_new(Box::new(fixture)).unwrap(),
            )]),
        },
    );

    // Encode: `validate_canonical_document` type-identity check + serialize.
    let encoded = codec.encode(&document).expect("encode set/map leaves");
    // Decode: rebuild `DynamicSet`/`DynamicMap`, hashing each opaque leaf.
    let round_trip = codec.decode(&encoded).expect("decode set/map leaves");

    let registration = registry
        .get(TypeId::of::<SetHashFixture>())
        .expect("SetHashFixture registration");
    let mut world = World::new();
    world.insert_resource(AppTypeRegistry::default());
    let entity_id = world.spawn_empty().id();
    let mut references = SceneEntityReferences::default();
    let built = {
        let mut entity_mut = world.entity_mut(entity_id);
        construct_reflected(
            registration,
            round_trip.entities[&entity("host")].components["SetHashFixture"].value(),
            &mut entity_mut,
            &mut references,
        )
        .expect("materialize set/map leaves")
    };
    let concrete = built
        .value
        .try_downcast_ref::<SetHashFixture>()
        .expect("typed SetHashFixture");
    assert_eq!(concrete.entities, entities);
    assert_eq!(concrete.crcs, crcs);
    assert_eq!(concrete.weights, weights);
}

#[test]
fn alias_and_type_version_migration_are_canonical_and_deterministic() {
    let registry = mesh_registry(migrate_mesh_v1_to_v2);
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"MeshComponent": 1},
        entities: {
            "crate": (components: {"MeshComponent": (mesh: "meshes/crate.azmesh")}),
        },
        instances: {},
    )"#;

    let document = codec.decode(source).expect("migrate Mesh alias");
    assert_eq!(
        document.type_versions,
        BTreeMap::from([("Mesh".to_owned(), 2)])
    );
    assert!(
        document.entities[&entity("crate")]
            .components
            .contains_key("Mesh")
    );
    let fields = document.entities[&entity("crate")].components["Mesh"]
        .value()
        .reflect_ref()
        .as_struct()
        .expect("Mesh sparse struct");
    assert_eq!(
        fields
            .field("visible")
            .and_then(|value| value.try_downcast_ref::<bool>()),
        Some(&true)
    );
    assert_eq!(
        fields
            .field("lod_bias")
            .and_then(|value| value.try_downcast_ref::<i32>()),
        Some(&0_i32)
    );

    let first = codec.encode(&document).expect("first encoding");
    let second = codec
        .encode(&codec.decode(&first).expect("canonical decode"))
        .expect("second encoding");
    assert_eq!(first, second);
    assert!(first.contains("\"Mesh\": 2"));
    assert!(!first.contains("MeshComponent"));
}

#[test]
fn codec_rejects_unknown_tags_fields_versions_and_post_migration_fields() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let unknown_tag = r#"(version: 1, type_versions: {"Missing": 1}, entities: {}, instances: {})"#;
    assert!(matches!(
        codec.decode(unknown_tag),
        Err(az_prefab::PrefabCodecError::Migration(
            PrefabMigrationError::UnknownTag(_)
        ))
    ));

    let unknown_field = r#"(
        version: 1,
        type_versions: {"CodecFixture": 1},
        entities: {"e": (components: {"CodecFixture": (missing: 1.0)})},
        instances: {},
    )"#;
    assert!(matches!(
        codec.decode(unknown_field),
        Err(az_prefab::PrefabCodecError::ReflectDeserialize { .. })
    ));

    let bad_root = format!(
        "(version: {}, type_versions: {{}}, entities: {{}}, instances: {{}})",
        99
    );
    assert!(matches!(
        codec.decode(&bad_root),
        Err(az_prefab::PrefabCodecError::UnsupportedDocumentVersion { .. })
    ));

    let registry = mesh_registry(migrate_with_unknown_field);
    let codec = PrefabCodec::new(&registry).expect("codec");
    let source = r#"(
        version: 1,
        type_versions: {"MeshComponent": 1},
        entities: {"e": (components: {"MeshComponent": (mesh: "mesh")})},
        instances: {},
    )"#;
    assert!(matches!(
        codec.decode(source),
        Err(az_prefab::PrefabCodecError::InvalidReflectedShape { .. })
    ));
}

#[test]
fn migration_registry_rejects_gapped_and_cyclic_chains() {
    // The signature is fixed by `PrefabMigrationStep::migrate`, so the
    // infallible body still has to return a `Result`.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "matches the PrefabMigrationStep::migrate function-pointer type"
    )]
    fn identity(value: ErasedPrefabValue) -> Result<ErasedPrefabValue, PrefabBuildError> {
        Ok(value)
    }

    let gap = PrefabMigrationStep {
        from_version: 1,
        to_version: 1 + 2,
        migrate: identity,
    };
    let registry = registry_with_steps(3, Box::leak(vec![gap].into_boxed_slice()));
    assert!(matches!(
        PrefabRegistry::try_new(&registry),
        Err(PrefabMigrationError::GappedMigration { .. })
    ));

    let cycle = PrefabMigrationStep {
        from_version: 1 + 1,
        to_version: 1,
        migrate: identity,
    };
    let registry = registry_with_steps(2, Box::leak(vec![cycle].into_boxed_slice()));
    assert!(matches!(
        PrefabRegistry::try_new(&registry),
        Err(PrefabMigrationError::CyclicMigration { .. })
    ));

    let registry = registry_with_steps(1 + 1, &[]);
    assert!(matches!(
        PrefabRegistry::try_new(&registry),
        Err(PrefabMigrationError::MissingMigrationStep { .. })
    ));
}

#[test]
fn override_ron_round_trip_preserves_named_target_and_explicit_value() {
    let registry = codec_registry();
    let codec = PrefabCodec::new(&registry).expect("codec");
    let component_type = CodecFixture::type_info().type_path();
    let source = format!(
        r#"(
            version: 1,
            type_versions: {{}},
            entities: {{}},
            instances: {{
                "weapon": (
                    source: "leaf.prefab.ron",
                    overrides: [(
                        target: (
                            instance_chain: ["inner"],
                            entity: "blade",
                            component: "{component_type}",
                            path: ".amount",
                        ),
                        action: Set(0.0),
                    )],
                ),
            }},
        )"#
    );

    let document = codec.decode(&source).expect("decode override");
    let operation = &document.instances[&instance("weapon")].overrides[0];
    assert_eq!(operation.target.instance_chain, vec![instance("inner")]);
    assert_eq!(operation.target.entity, entity("blade"));
    assert_eq!(operation.target.path.to_string(), ".amount");
    let TypedOverrideAction::Set(value) = &operation.action else {
        panic!("expected Set action");
    };
    assert_eq!(value.value().try_downcast_ref::<f32>(), Some(&0.0));

    let encoded = codec.encode(&document).expect("encode override");
    let round_trip = codec.decode(&encoded).expect("round trip override");
    let TypedOverrideAction::Set(value) =
        &round_trip.instances[&instance("weapon")].overrides[0].action
    else {
        panic!("expected Set action");
    };
    assert_eq!(value.value().try_downcast_ref::<f32>(), Some(&0.0));
    assert!(encoded.contains("path: \".amount\""));
}

#[test]
fn typed_semantics_reject_alias_hierarchy_and_dependency_cycles() {
    let registry = codec_registry();
    let mut document = PrefabDocument::default();
    document
        .entities
        .insert(entity("same"), PrefabEntity::default());
    document.instances.insert(
        instance("same"),
        PrefabInstance {
            source: asset("other.prefab.ron"),
            parent: None,
            overrides: Vec::new(),
        },
    );
    assert!(matches!(
        TypedPrefabSemantics::validate_local(&document, &registry),
        Err(TypedPrefabSemanticsError::AliasCollision(_))
    ));

    let mut hierarchy = PrefabDocument::default();
    hierarchy.entities.insert(
        entity("a"),
        PrefabEntity {
            entity_id: None,
            parent: Some(entity("b")),
            components: BTreeMap::new(),
        },
    );
    hierarchy.entities.insert(
        entity("b"),
        PrefabEntity {
            entity_id: None,
            parent: Some(entity("a")),
            components: BTreeMap::new(),
        },
    );
    assert!(matches!(
        TypedPrefabSemantics::validate_local(&hierarchy, &registry),
        Err(TypedPrefabSemanticsError::HierarchyCycle { .. })
    ));

    let path_a = asset("a.prefab.ron");
    let path_b = asset("b.prefab.ron");
    let mut a = PrefabDocument::default();
    a.instances
        .insert(instance("b"), empty_instance(path_b.clone()));
    let mut b = PrefabDocument::default();
    b.instances
        .insert(instance("a"), empty_instance(path_a.clone()));
    let resolver = BTreeMap::from([(path_a.clone(), a.clone()), (path_b, b)]);
    assert!(matches!(
        TypedPrefabSemantics::dependency_order(&path_a, &a, &resolver),
        Err(TypedPrefabSemanticsError::DependencyCycle { .. })
    ));
}

#[test]
fn nested_override_precedence_is_inner_to_outer_and_same_layer_conflicts() {
    let registry = codec_registry();
    let component_type = CodecFixture::type_info().type_path().to_owned();
    let leaf_path = asset("leaf.prefab.ron");
    let middle_path = asset("middle.prefab.ron");
    let root_path = asset("root.prefab.ron");

    let mut leaf = document_with_component("blade");
    leaf.type_versions.insert("CodecFixture".to_owned(), 1);

    let mut middle = PrefabDocument::default();
    middle.instances.insert(
        instance("blade_instance"),
        PrefabInstance {
            source: leaf_path.clone(),
            parent: None,
            overrides: vec![set_override(
                Vec::new(),
                "blade",
                &component_type,
                ".amount",
                1.0_f32,
            )],
        },
    );

    let mut root = PrefabDocument::default();
    root.instances.insert(
        instance("weapon"),
        PrefabInstance {
            source: middle_path.clone(),
            parent: None,
            overrides: vec![set_override(
                vec![instance("blade_instance")],
                "blade",
                &component_type,
                ".amount",
                2.0_f32,
            )],
        },
    );
    let documents = BTreeMap::from([(leaf_path, leaf), (middle_path, middle.clone())]);
    let resolved =
        TypedPrefabSemantics::resolve_overrides(&root_path, &root, &documents, &registry)
            .expect("resolve nested overrides");
    assert_eq!(resolved.len(), 1);
    assert_eq!(
        resolved[0]
            .target
            .instance_chain
            .iter()
            .map(InstanceAlias::as_str)
            .collect::<Vec<_>>(),
        vec!["weapon", "blade_instance"]
    );
    let TypedOverrideAction::Set(value) = &resolved[0].action else {
        panic!("expected Set override");
    };
    assert_eq!(value.value().try_downcast_ref::<f32>(), Some(&2.0));

    root.instances
        .get_mut(&instance("weapon"))
        .unwrap()
        .overrides
        .push(set_override(
            vec![instance("blade_instance")],
            "blade",
            &component_type,
            ".amount",
            3.0_f32,
        ));
    assert!(matches!(
        TypedPrefabSemantics::resolve_overrides(&root_path, &root, &documents, &registry),
        Err(TypedPrefabSemanticsError::SameLayerConflict { .. })
    ));
}

#[test]
fn typed_semantics_validate_stable_named_override_paths_and_dependency_order() {
    let registry = codec_registry();
    let component_type = CodecFixture::type_info().type_path().to_owned();
    let leaf_path = asset("leaf.prefab.ron");
    let middle_path = asset("middle.prefab.ron");
    let root_path = asset("root.prefab.ron");
    let leaf = document_with_component("target");
    let mut middle = PrefabDocument::default();
    middle
        .instances
        .insert(instance("leaf"), empty_instance(leaf_path.clone()));
    let mut root = PrefabDocument::default();
    root.instances.insert(
        instance("middle"),
        PrefabInstance {
            source: middle_path.clone(),
            parent: None,
            overrides: vec![set_override(
                vec![instance("leaf")],
                "target",
                &component_type,
                ".enabled",
                true,
            )],
        },
    );
    let resolver = BTreeMap::from([(leaf_path.clone(), leaf), (middle_path.clone(), middle)]);
    let dependencies = TypedPrefabSemantics::dependency_order(&root_path, &root, &resolver)
        .expect("dependency order");
    assert_eq!(dependencies, vec![leaf_path, middle_path]);
    TypedPrefabSemantics::validate(&root_path, &root, &resolver, &registry)
        .expect("valid named override path");

    root.instances
        .get_mut(&instance("middle"))
        .unwrap()
        .overrides[0]
        .target
        .path = ReflectedPath::parse(".missing").unwrap();
    assert!(matches!(
        TypedPrefabSemantics::validate(&root_path, &root, &resolver, &registry),
        Err(TypedPrefabSemanticsError::InvalidOverridePath { .. })
    ));
}

fn registry_with_steps(
    source_version: u32,
    migrations: &'static [PrefabMigrationStep],
) -> TypeRegistry {
    let mut registry = TypeRegistry::default();
    registry.register::<CodecFixture>();
    let registration = registry
        .get_mut(TypeId::of::<CodecFixture>())
        .expect("Codec fixture registration");
    registration.insert(PrefabTypeData {
        tag: "ManualFixture",
        source_version,
        aliases: &[],
        migrations,
        construction: PrefabConstruction::ReflectDefaultOrFromWorld,
        product_policy: PrefabProductPolicy::Runtime,
        construct: construct_reflected,
        insert: insert_reflected_component,
    });
    registry
}

fn document_with_component(alias: &str) -> PrefabDocument {
    let mut sparse = DynamicStruct::default();
    sparse.set_represented_type(Some(CodecFixture::type_info()));
    let mut document = PrefabDocument::default();
    document.type_versions.insert("CodecFixture".to_owned(), 1);
    document.entities.insert(
        entity(alias),
        PrefabEntity {
            entity_id: None,
            parent: None,
            components: BTreeMap::from([(
                "CodecFixture".to_owned(),
                SparseValue::try_new(Box::new(sparse)).unwrap(),
            )]),
        },
    );
    document
}

fn set_override<T: PartialReflect>(
    instance_chain: Vec<InstanceAlias>,
    entity_alias: &str,
    component: &str,
    path: &str,
    value: T,
) -> OverrideOperation {
    OverrideOperation {
        target: TypedOverrideTarget::new(
            instance_chain,
            entity(entity_alias),
            component,
            ReflectedPath::parse(path).unwrap(),
        )
        .unwrap(),
        action: TypedOverrideAction::Set(SparseValue::try_new(Box::new(value)).unwrap()),
    }
}

const fn empty_instance(source: PrefabAssetPath) -> PrefabInstance {
    PrefabInstance {
        source,
        parent: None,
        overrides: Vec::new(),
    }
}

fn entity(value: &str) -> EntityAlias {
    EntityAlias::new(value).unwrap()
}

fn instance(value: &str) -> InstanceAlias {
    InstanceAlias::new(value).unwrap()
}

fn asset(value: &str) -> PrefabAssetPath {
    PrefabAssetPath::new(value).unwrap()
}
