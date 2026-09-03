use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    rc::Rc,
};

use az_core::{
    EditorActionId, EditorActionOutcome, EditorFieldAttributes, EditorFieldConstraints,
    EditorPolicyError, EditorPolicyTypeData, EditorTypeAttributes, EditorWidget,
    ReflectedPath as CoreReflectedPath, ReflectedPathSegment as CorePathSegment,
};
use az_gem_contract::{
    Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
    GemTargetRole, ProductActivation, declare_caps,
};
use az_prefab::{
    EntityAlias, InstanceAlias, OverrideOperation, PREFAB_DOCUMENT_VERSION, Prefab,
    PrefabAssetPath, PrefabCodec, PrefabDocument, PrefabEntity, PrefabInstance, PrefabType,
    ReflectPrefab, ReflectedPath as PrefabReflectedPath, SparseValue, TypedOverrideAction,
    TypedOverrideTarget,
};
use az_proto_project::{
    project_capnp,
    vnext::{
        PrefabEditCommand, PrefabOverrideOperation, PrefabRpcResult, PrefabSourceSnapshot,
        PrefabValueTarget, ReflectedPath, ReflectedPathSegment, ReflectedTypeDescriptor,
        ReflectedTypeKind, ReflectedValueEncoding, ReflectedValueEnvelope, SourceSessionCommand,
        SourceSessionResult, TypeRegistrySnapshot, TypedActionResult,
    },
};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{
    PartialReflect, Reflect, TypePath, Typed, std_traits::ReflectDefault, structs::DynamicStruct,
};

use super::*;

const BASELINE_SOURCE: &str = "component-baseline.prefab.ron";
const NESTED_SOURCE: &str = "nested-override.prefab.ron";
const INVALID_SOURCE: &str = "invalid-validation.prefab.ron";
const VARIANT_SOURCE: &str = "variant-shapes.prefab.ron";
const ACTION_ID: &str = "vnext.reset_scalar";

#[derive(Debug, Clone, Default, PartialEq, Reflect)]
enum HarnessMode {
    #[default]
    Alpha,
    Beta,
}

#[derive(Debug, Clone, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("vNext RPC Harness")
    .in_group("ADR 0022")
    .with_action(ACTION_ID))]
#[prefab(tag = "VNextHarness", version = 1)]
struct VNextHarnessComponent {
    #[reflect(@EditorFieldAttributes::new("Scalar", EditorWidget::Number))]
    scalar: f32,
    #[reflect(@EditorFieldAttributes::new("Items", EditorWidget::Default))]
    items: Vec<f32>,
    #[reflect(@EditorFieldAttributes::new("Values", EditorWidget::Default))]
    values: BTreeMap<String, f32>,
    #[reflect(@EditorFieldAttributes::new("Mode", EditorWidget::Default))]
    mode: HarnessMode,
    #[reflect(@EditorFieldAttributes::new(
        "Notes",
        EditorWidget::Multiline { rows: Some(4) },
    ).with_constraints(EditorFieldConstraints {
        minimum_length: Some(1),
        maximum_length: Some(256),
        allowed_strings: vec!["line one\nline two".to_owned()],
        allowed_variants: Vec::new(),
    }))]
    notes: String,
    #[reflect(@EditorFieldAttributes {
        label: Some("Locked".to_owned()),
        read_only: true,
        ..EditorFieldAttributes::default()
    })]
    locked: bool,
    #[reflect(@EditorFieldAttributes {
        label: Some("Internal".to_owned()),
        hidden: true,
        ..EditorFieldAttributes::default()
    })]
    internal: bool,
}

impl Default for VNextHarnessComponent {
    fn default() -> Self {
        Self {
            scalar: 1.0,
            items: vec![1.0, 2.0],
            values: BTreeMap::from([("initial".to_owned(), 1.0)]),
            mode: HarnessMode::Alpha,
            notes: "line one\nline two".to_owned(),
            locked: true,
            internal: false,
        }
    }
}

/// Carries one variant of every shape the producer can emit, so a `SetVariant`
/// command naming a variant and nothing else has a declared shape to honour.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Default)]
enum VariantShapeMode {
    #[default]
    Marker,
    Fieldless(),
    Single(f32),
    Pair(f32, bool),
    Named {
        alpha: f32,
        beta: bool,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "VariantShape", version = 1)]
struct VariantShapeComponent {
    mode: VariantShapeMode,
}

declare_caps!(HarnessCaps:);

/// Stands in for a gem's runtime contribution.
struct Harness;

impl Contribution for Harness {
    type Caps = HarnessCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.project-host-vnext-tests"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, HarnessCaps>) {
        ctx.registrar::<PrefabType>().register_many([
            PrefabType::of::<HarnessMode>(),
            PrefabType::of::<Vec<f32>>(),
            PrefabType::of::<BTreeMap<String, f32>>(),
            PrefabType::of::<VNextHarnessComponent>(),
            PrefabType::of::<VariantShapeMode>(),
            PrefabType::of::<VariantShapeComponent>(),
        ]);
    }
}

fn harness_composition() -> Composition {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    crate::tests::floor(&mut composer);
    composer
        .add(Harness, ProductActivation::default())
        .expect("an empty capability floor composes");
    Composition::new(composer).expect("ProjectHost harness composition is valid and ready")
}

/// The editor action callback: a fn pointer, so no [`PrefabType`] can carry
/// it. Reflected type data with no composed carrier is inserted into the
/// composed registry by the harness that needs it.
fn insert_harness_policy(registry: &ComposedTypeRegistry) {
    registry
        .app_registry
        .write()
        .get_mut(TypeId::of::<VNextHarnessComponent>())
        .expect("harness component registration")
        .insert(EditorPolicyTypeData {
            invoke_action: Some(invoke_harness_action),
            ..EditorPolicyTypeData::default()
        });
}

fn invoke_harness_action(
    value: &mut dyn PartialReflect,
    action_id: &EditorActionId,
) -> Result<EditorActionOutcome, EditorPolicyError> {
    if action_id.0 != ACTION_ID {
        return Err(EditorPolicyError::UnknownAction(action_id.0.clone()));
    }
    let reflected_type_path = value.reflect_type_path().to_owned();
    let value = value
        .try_downcast_mut::<VNextHarnessComponent>()
        .ok_or(EditorPolicyError::IncompatibleValue(reflected_type_path))?;
    value.scalar = 0.0;
    Ok(EditorActionOutcome {
        changed_paths: vec![CoreReflectedPath(vec![CorePathSegment::Field(
            "scalar".to_owned(),
        )])],
        diagnostics: Vec::new(),
    })
}

struct RpcHarness {
    client: project_capnp::project_host::Client,
    _rpc: Rc<ProjectHostRpc>,
    _composition: Composition,
    _temp: tempfile::TempDir,
}

impl RpcHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("vNext RPC source root");
        az_project::write_project_manifest(
            temp.path(),
            &az_project::ProjectManifest::new(
                "local.az_project_host_vnext",
                "Project Host vNext Test",
                "0.1.0",
            ),
        )
        .expect("write vNext RPC project manifest");
        let composition = harness_composition();
        let registry =
            compose_type_registry(composition.prefabs()).expect("vNext harness registry");
        insert_harness_policy(&registry);
        write_typed_source(
            temp.path().join(BASELINE_SOURCE),
            &baseline_document(),
            &registry,
        );
        write_typed_source(
            temp.path().join(NESTED_SOURCE),
            &nested_override_document(),
            &registry,
        );
        write_typed_source(
            temp.path().join(INVALID_SOURCE),
            &invalid_validation_document(),
            &registry,
        );
        write_typed_source(
            temp.path().join(VARIANT_SOURCE),
            &variant_shape_document(),
            &registry,
        );

        let rpc = Rc::new(ProjectHostRpc::test_new_composed(
            ProjectHost::with_source_root(temp.path()),
            test_capability_grants(),
            &composition,
        ));
        insert_harness_policy(&rpc.registry());
        let client = ProjectHostRpc::client_from_rc(&rpc);
        Self {
            client,
            _rpc: rpc,
            _composition: composition,
            _temp: temp,
        }
    }

    fn open(&self, source: &str) -> SourceSessionResult {
        lifecycle(&self.client, source, SourceSessionCommand::Open, 0)
    }
}

fn write_typed_source(path: PathBuf, document: &PrefabDocument, registry: &ComposedTypeRegistry) {
    let registry = registry.app_registry.read();
    let source = PrefabCodec::new(&registry)
        .and_then(|codec| codec.encode(document))
        .expect("encode vNext typed Prefab fixture");
    drop(registry);
    fs::write(path, source).expect("write vNext typed Prefab fixture");
}

fn baseline_document() -> PrefabDocument {
    let mut transform = dynamic_struct::<az_transform::Transform>();
    transform.insert("position", glam::Vec3::new(2.0, 3.0, 4.0));
    let mut camera = dynamic_struct::<az_render::Camera>();
    camera.insert("near", 0.1_f32);

    PrefabDocument {
        version: PREFAB_DOCUMENT_VERSION,
        type_versions: BTreeMap::from([
            ("Transform".to_owned(), 1),
            ("Mesh".to_owned(), 2),
            ("MaterialAssignment".to_owned(), 1),
            ("Camera".to_owned(), 1),
            ("VNextHarness".to_owned(), 1),
        ]),
        catalog_aliases: BTreeSet::new(),
        entities: BTreeMap::from([(
            alias("fixture-root"),
            PrefabEntity {
                entity_id: None,
                parent: None,
                components: BTreeMap::from([
                    ("Transform".to_owned(), sparse(transform)),
                    (
                        "Mesh".to_owned(),
                        sparse(dynamic_struct::<az_render::Mesh>()),
                    ),
                    (
                        "MaterialAssignment".to_owned(),
                        sparse(dynamic_struct::<az_render::MaterialAssignment>()),
                    ),
                    ("Camera".to_owned(), sparse(camera)),
                    (
                        "VNextHarness".to_owned(),
                        sparse(VNextHarnessComponent::default()),
                    ),
                ]),
            },
        )]),
        instances: BTreeMap::new(),
    }
}

fn variant_shape_document() -> PrefabDocument {
    PrefabDocument {
        version: PREFAB_DOCUMENT_VERSION,
        type_versions: BTreeMap::from([("VariantShape".to_owned(), 1)]),
        catalog_aliases: BTreeSet::new(),
        entities: BTreeMap::from([(
            alias("variant-root"),
            PrefabEntity {
                entity_id: None,
                parent: None,
                components: BTreeMap::from([(
                    "VariantShape".to_owned(),
                    sparse(VariantShapeComponent::default()),
                )]),
            },
        )]),
        instances: BTreeMap::new(),
    }
}

fn nested_override_document() -> PrefabDocument {
    let target = |component: String, path: &str| {
        TypedOverrideTarget::new(
            Vec::new(),
            alias("base-camera"),
            component,
            PrefabReflectedPath::new([path]).expect("override path"),
        )
        .expect("override target")
    };
    PrefabDocument {
        version: PREFAB_DOCUMENT_VERSION,
        type_versions: BTreeMap::new(),
        catalog_aliases: BTreeSet::new(),
        entities: BTreeMap::from([(
            alias("mount"),
            PrefabEntity {
                entity_id: None,
                parent: None,
                components: BTreeMap::new(),
            },
        )]),
        instances: BTreeMap::from([(
            InstanceAlias::new("door-instance").expect("instance alias"),
            PrefabInstance {
                source: PrefabAssetPath::new("prefabs/base-door.prefab.ron")
                    .expect("instance source"),
                parent: Some(alias("mount")),
                overrides: vec![
                    OverrideOperation {
                        target: target(type_path::<az_render::Camera>(), "near"),
                        action: TypedOverrideAction::Set(sparse(2.0_f32)),
                    },
                    OverrideOperation {
                        target: target(type_path::<az_render::Camera>(), "far"),
                        action: TypedOverrideAction::Clear,
                    },
                    OverrideOperation {
                        target: target(type_path::<VNextHarnessComponent>(), "items"),
                        action: TypedOverrideAction::Insert {
                            index: 1,
                            value: sparse(3.0_f32),
                        },
                    },
                    OverrideOperation {
                        target: target(type_path::<VNextHarnessComponent>(), "items"),
                        action: TypedOverrideAction::Remove { index: 2 },
                    },
                    OverrideOperation {
                        target: target(type_path::<VNextHarnessComponent>(), "items"),
                        action: TypedOverrideAction::Move { from: 3, to: 4 },
                    },
                ],
            },
        )]),
    }
}

fn invalid_validation_document() -> PrefabDocument {
    let mut camera = dynamic_struct::<az_render::Camera>();
    camera.insert("near", 2.0_f32);
    camera.insert("far", 1.0_f32);
    let mut spot = dynamic_struct::<az_render::SpotLight>();
    spot.insert("inner_angle_degrees", 90.0_f32);
    spot.insert("outer_angle_degrees", 45.0_f32);
    PrefabDocument {
        version: PREFAB_DOCUMENT_VERSION,
        type_versions: BTreeMap::from([("Camera".to_owned(), 1), ("SpotLight".to_owned(), 1)]),
        catalog_aliases: BTreeSet::new(),
        entities: BTreeMap::from([(
            alias("invalid"),
            PrefabEntity {
                entity_id: None,
                parent: None,
                components: BTreeMap::from([
                    ("Camera".to_owned(), sparse(camera)),
                    ("SpotLight".to_owned(), sparse(spot)),
                ]),
            },
        )]),
        instances: BTreeMap::new(),
    }
}

fn dynamic_struct<T: Typed>() -> DynamicStruct {
    let mut value = DynamicStruct::default();
    value.set_represented_type(Some(T::type_info()));
    value
}

fn sparse(value: impl PartialReflect + 'static) -> SparseValue {
    SparseValue::try_new(Box::new(value)).expect("represented sparse value")
}

fn alias(value: &str) -> EntityAlias {
    EntityAlias::new(value).expect("entity alias")
}

fn type_path<T: TypePath>() -> String {
    T::type_path().to_owned()
}

fn envelope<T: TypePath>(ron: &str) -> ReflectedValueEnvelope {
    ReflectedValueEnvelope {
        type_path: type_path::<T>(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload: ron.as_bytes().to_vec(),
    }
}

fn component_target<T: TypePath>(
    entity: &str,
    segments: Vec<ReflectedPathSegment>,
) -> PrefabValueTarget {
    PrefabValueTarget {
        instance_alias_chain: Vec::new(),
        entity_alias: entity.to_owned(),
        path: ReflectedPath {
            component_type_path: type_path::<T>(),
            segments,
        },
    }
}

fn registry_snapshot(client: &project_capnp::project_host::Client) -> TypeRegistrySnapshot {
    let mut request = client.type_registry_snapshot_request();
    write_test_capability(request.get().init_capability(), [PROJECT_SCHEMA_PERMISSION]);
    let response =
        futures::executor::block_on(request.send().promise).expect("type registry RPC response");
    az_proto_project::vnext::TypeRegistrySnapshot::from_capnp(
        response
            .get()
            .expect("type registry RPC result")
            .get_snapshot()
            .expect("type registry snapshot"),
    )
    .expect("decode type registry snapshot")
}

fn lifecycle(
    client: &project_capnp::project_host::Client,
    source: &str,
    command: SourceSessionCommand,
    expected_revision: u64,
) -> SourceSessionResult {
    let mut request = client.source_session_lifecycle_request();
    {
        let mut request = request.get();
        let permission = if matches!(
            command,
            SourceSessionCommand::Open | SourceSessionCommand::Status
        ) {
            PROJECT_DOCUMENT_READ_PERMISSION
        } else {
            PROJECT_DOCUMENT_WRITE_PERMISSION
        };
        write_test_capability(request.reborrow().init_capability(), [permission]);
        request.set_source_path(source);
        request.set_command((command).to_capnp());
        request.set_expected_revision(expected_revision);
    }
    let response =
        futures::executor::block_on(request.send().promise).expect("source lifecycle RPC response");
    az_proto_project::vnext::SourceSessionResult::from_capnp(
        response
            .get()
            .expect("source lifecycle RPC result")
            .get_result()
            .expect("source lifecycle result"),
    )
    .expect("decode source lifecycle result")
}

fn prefab_snapshot(client: &project_capnp::project_host::Client, source: &str) -> PrefabRpcResult {
    let mut request = client.prefab_source_snapshot_request();
    {
        let mut request = request.get();
        write_test_capability(
            request.reborrow().init_capability(),
            [PROJECT_DOCUMENT_READ_PERMISSION],
        );
        request.set_source_path(source);
    }
    let response =
        futures::executor::block_on(request.send().promise).expect("Prefab snapshot RPC response");
    az_proto_project::vnext::PrefabRpcResult::from_capnp(
        response
            .get()
            .expect("Prefab snapshot RPC result")
            .get_result()
            .expect("Prefab snapshot result"),
    )
    .expect("decode Prefab snapshot result")
}

fn apply_edit(
    client: &project_capnp::project_host::Client,
    source: &str,
    expected_revision: u64,
    command: &PrefabEditCommand,
) -> PrefabRpcResult {
    let mut request = client.apply_prefab_edit_command_request();
    {
        let mut request = request.get();
        write_test_capability(
            request.reborrow().init_capability(),
            [PROJECT_EDIT_PERMISSION],
        );
        request.set_source_path(source);
        request.set_expected_revision(expected_revision);
        (command)
            .to_capnp(request.init_command())
            .expect("encode Prefab edit command");
    }
    let response =
        futures::executor::block_on(request.send().promise).expect("Prefab edit RPC response");
    az_proto_project::vnext::PrefabRpcResult::from_capnp(
        response
            .get()
            .expect("Prefab edit RPC result")
            .get_result()
            .expect("Prefab edit result"),
    )
    .expect("decode Prefab edit result")
}

fn invoke_action(
    client: &project_capnp::project_host::Client,
    source: &str,
    expected_revision: u64,
    target: &PrefabValueTarget,
) -> TypedActionResult {
    let mut request = client.invoke_typed_action_request();
    {
        let mut request = request.get();
        write_test_capability(
            request.reborrow().init_capability(),
            [PROJECT_EDIT_PERMISSION],
        );
        request.set_source_path(source);
        request.set_expected_revision(expected_revision);
        request.set_action_id(ACTION_ID);
        (target)
            .to_capnp(request.init_target())
            .expect("encode typed action target");
    }
    let response =
        futures::executor::block_on(request.send().promise).expect("typed action RPC response");
    az_proto_project::vnext::TypedActionResult::from_capnp(
        response
            .get()
            .expect("typed action RPC result")
            .get_result()
            .expect("typed action result"),
    )
    .expect("decode typed action result")
}

fn diagnostics(
    client: &project_capnp::project_host::Client,
    source: &str,
) -> Vec<az_proto_project::vnext::PrefabDiagnostic> {
    let mut request = client.prefab_diagnostics_request();
    {
        let mut request = request.get();
        write_test_capability(
            request.reborrow().init_capability(),
            [PROJECT_DOCUMENT_READ_PERMISSION],
        );
        request.set_source_path(source);
    }
    let response = futures::executor::block_on(request.send().promise)
        .expect("Prefab diagnostics RPC response");
    response
        .get()
        .expect("Prefab diagnostics RPC result")
        .get_diagnostics()
        .expect("Prefab diagnostics")
        .iter()
        .map(az_proto_project::vnext::PrefabDiagnostic::from_capnp)
        .collect::<Result<Vec<_>, _>>()
        .expect("decode Prefab diagnostics")
}

fn descriptor<'a>(
    snapshot: &'a TypeRegistrySnapshot,
    type_path: &str,
) -> &'a ReflectedTypeDescriptor {
    snapshot
        .types
        .iter()
        .find(|descriptor| descriptor.type_path == type_path)
        .unwrap_or_else(|| panic!("missing reflected type `{type_path}`"))
}

fn field<'a>(
    descriptor: &'a ReflectedTypeDescriptor,
    name: &str,
) -> &'a az_proto_project::vnext::ReflectedFieldDescriptor {
    descriptor
        .fields
        .iter()
        .find(|field| field.name == name)
        .unwrap_or_else(|| panic!("missing reflected field `{name}`"))
}

fn component<'a, T: TypePath>(
    snapshot: &'a PrefabSourceSnapshot,
    entity: &str,
) -> &'a az_proto_project::vnext::PrefabComponentSnapshot {
    let type_path = type_path::<T>();
    snapshot
        .components
        .iter()
        .find(|component| component.entity_alias == entity && component.type_path == type_path)
        .unwrap_or_else(|| panic!("missing `{type_path}` on `{entity}`"))
}

fn snapshot_from(result: PrefabRpcResult) -> PrefabSourceSnapshot {
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.snapshot.expect("successful Prefab snapshot")
}

/// Assert the harness scalar came back exactly as authored.
///
/// Every literal these tests pin (`0.0`, `1.0`, `4.5`, `8.0`) is exactly
/// representable in binary `f32` and round-trips through the real codec
/// unchanged, so exactness *is* the assertion: an epsilon comparison would stop
/// catching a lossy encode, which is the regression this guards.
#[allow(clippy::float_cmp)]
fn assert_harness_scalar(snapshot: &PrefabSourceSnapshot, expected: f32) {
    assert_eq!(
        harness_scalar(snapshot),
        expected,
        "harness scalar must round-trip exactly"
    );
}

fn harness_scalar(snapshot: &PrefabSourceSnapshot) -> f32 {
    let composition = harness_composition();
    let registry = compose_type_registry(composition.prefabs()).expect("vNext harness registry");
    let registry = registry.app_registry.read();
    let component = component::<VNextHarnessComponent>(snapshot, "fixture-root");
    let sparse = PrefabCodec::new(&registry)
        .and_then(|codec| {
            codec.decode_sparse_value(
                &component.sparse_value.type_path,
                &component.sparse_value.payload,
            )
        })
        .expect("decode harness component envelope");
    drop(registry);
    *sparse
        .value()
        .reflect_ref()
        .as_struct()
        .expect("harness struct")
        .field("scalar")
        .expect("harness scalar")
        .try_downcast_ref::<f32>()
        .expect("f32 harness scalar")
}

#[test]
fn vnext_rpc_registry_covers_slider_range_color_vector_enum_nested_asset_and_static_policy_metadata()
 {
    let harness = RpcHarness::new();
    let snapshot = registry_snapshot(&harness.client);
    assert_eq!(snapshot.schema_catalog_hash.len(), 32);

    assert_transform_descriptor(&snapshot);
    assert_mesh_descriptor(&snapshot);
    assert_camera_descriptors(&snapshot);
    assert_light_descriptors(&snapshot);
    assert_harness_descriptor(&snapshot);
}

fn assert_transform_descriptor(snapshot: &TypeRegistrySnapshot) {
    let transform = descriptor(snapshot, &type_path::<az_transform::Transform>());
    assert_eq!(transform.kind, ReflectedTypeKind::Struct);
    assert_eq!(
        transform.editor_attributes.label.as_deref(),
        Some("Transform")
    );
    assert_eq!(
        transform.editor_attributes.category.as_deref(),
        Some("Core")
    );
    assert_eq!(
        transform.editor_attributes.icon.as_deref(),
        Some("open_with")
    );
    assert_eq!(transform.applicability.provides, ["azoth.transform"]);
    assert!(transform.applicability.requires.is_empty());
    assert_eq!(transform.applicability.incompatible, ["azoth.transform"]);
    assert!(transform.applicability.default_available);
    assert!(transform.reflected_default.is_some());
    assert_eq!(
        transform
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["position", "rotation", "scale"]
    );
    assert_eq!(
        field(transform, "position")
            .editor_attributes
            .widget
            .as_deref(),
        Some("vector:3")
    );
    assert_eq!(
        field(transform, "position")
            .editor_attributes
            .range
            .as_ref()
            .and_then(|range| range.suffix.as_deref()),
        Some("m")
    );
}

fn assert_mesh_descriptor(snapshot: &TypeRegistrySnapshot) {
    let mesh = descriptor(snapshot, &type_path::<az_render::Mesh>());
    assert_eq!(
        field(mesh, "mesh").editor_attributes.widget.as_deref(),
        Some("asset:mesh")
    );
    let lod = field(mesh, "lod_bias")
        .editor_attributes
        .range
        .as_ref()
        .expect("LOD range");
    assert_eq!(
        (
            lod.minimum.as_deref(),
            lod.maximum.as_deref(),
            lod.step.as_deref()
        ),
        (Some("-2"), Some("2"), Some("1"))
    );
}

fn assert_camera_descriptors(snapshot: &TypeRegistrySnapshot) {
    let camera = descriptor(snapshot, &type_path::<az_render::Camera>());
    assert_eq!(camera.applicability.requires, ["azoth.transform"]);
    assert!(
        camera
            .type_data_flags
            .iter()
            .any(|flag| flag == "Validation")
    );
    assert_eq!(
        field(camera, "near")
            .editor_attributes
            .range
            .as_ref()
            .and_then(|range| range.suffix.as_deref()),
        Some("m")
    );
    let projection = descriptor(snapshot, &type_path::<az_render::CameraProjection>());
    assert_eq!(projection.kind, ReflectedTypeKind::Enum);
    assert_eq!(
        projection
            .variants
            .iter()
            .map(|variant| variant.name.as_str())
            .collect::<Vec<_>>(),
        ["Perspective", "Orthographic"]
    );
}

fn assert_light_descriptors(snapshot: &TypeRegistrySnapshot) {
    let directional = descriptor(snapshot, &type_path::<az_render::DirectionalLight>());
    assert_eq!(directional.applicability.requires, ["azoth.transform"]);
    assert_eq!(
        field(directional, "color")
            .editor_attributes
            .widget
            .as_deref(),
        Some("color")
    );
    assert_eq!(
        field(directional, "illuminance_lux")
            .editor_attributes
            .range
            .as_ref()
            .and_then(|range| range.suffix.as_deref()),
        Some("lux")
    );
    let point = descriptor(snapshot, &type_path::<az_render::PointLight>());
    assert_eq!(point.applicability.requires, ["azoth.transform"]);
    assert_eq!(
        field(point, "intensity_lumens")
            .editor_attributes
            .range
            .as_ref()
            .and_then(|range| range.suffix.as_deref()),
        Some("lm")
    );
    let spot = descriptor(snapshot, &type_path::<az_render::SpotLight>());
    assert_eq!(spot.applicability.requires, ["azoth.transform"]);
    assert_eq!(
        field(spot, "outer_angle_degrees")
            .editor_attributes
            .range
            .as_ref()
            .and_then(|range| range.suffix.as_deref()),
        Some("°")
    );
}

fn assert_harness_descriptor(snapshot: &TypeRegistrySnapshot) {
    let custom = descriptor(snapshot, &type_path::<VNextHarnessComponent>());
    assert_eq!(custom.editor_attributes.action_ids, [ACTION_ID]);
    assert!(
        custom
            .type_data_flags
            .iter()
            .any(|flag| flag == "EditorPolicy")
    );
    assert_eq!(
        field(custom, "notes").editor_attributes.widget.as_deref(),
        Some("multiline:4")
    );
    assert_eq!(
        field(custom, "notes")
            .editor_attributes
            .constraints
            .minimum_length,
        Some(1)
    );
    assert_eq!(
        field(custom, "notes")
            .editor_attributes
            .constraints
            .maximum_length,
        Some(256)
    );
    assert!(field(custom, "locked").editor_attributes.read_only);
    assert!(field(custom, "internal").editor_attributes.hidden);
}

#[test]
fn vnext_rpc_prefab_source_snapshot_preserves_typed_sparse_baseline_and_nested_override_intent() {
    let legacy_baseline = include_str!(
        "../../../../editor/inspector/tests/fixtures/adr0022/sources/component-baseline.prefab.ron"
    );
    let legacy_nested = include_str!(
        "../../../../editor/inspector/tests/fixtures/adr0022/sources/nested-override.prefab.ron"
    );
    assert!(legacy_baseline.contains("azoth.render.Camera"));
    assert!(legacy_nested.contains("prefabs/base-door.prefab.ron"));

    let harness = RpcHarness::new();
    let opened = harness.open(BASELINE_SOURCE);
    assert!(opened.status.open);
    assert!(opened.diagnostics.is_empty(), "{:?}", opened.diagnostics);
    let snapshot = snapshot_from(prefab_snapshot(&harness.client, BASELINE_SOURCE));
    assert_eq!(snapshot.entities[0].alias, "fixture-root");
    assert_eq!(snapshot.components.len(), 5);
    let transform = component::<az_transform::Transform>(&snapshot, "fixture-root");
    assert!(String::from_utf8_lossy(&transform.sparse_value.payload).contains("position"));
    let camera = component::<az_render::Camera>(&snapshot, "fixture-root");
    let camera_payload = String::from_utf8_lossy(&camera.sparse_value.payload);
    assert!(camera_payload.contains("near"));
    assert!(!camera_payload.contains("far"));

    let nested = harness.open(NESTED_SOURCE);
    assert!(nested.diagnostics.is_empty(), "{:?}", nested.diagnostics);
    let snapshot = snapshot_from(prefab_snapshot(&harness.client, NESTED_SOURCE));
    assert_eq!(snapshot.instances[0].alias, "door-instance");
    assert_eq!(
        snapshot.instances[0].parent_entity_alias.as_deref(),
        Some("mount")
    );
    assert_eq!(snapshot.overrides.len(), 5);
    assert_eq!(
        snapshot.overrides[0]
            .operation
            .target()
            .instance_alias_chain,
        ["door-instance"]
    );
    assert_eq!(
        snapshot.overrides[0].operation.target().entity_alias,
        "base-camera"
    );
    assert_eq!(
        snapshot.overrides[0].operation.target().path.segments,
        [ReflectedPathSegment::Field("near".to_owned())]
    );
    assert!(matches!(
        snapshot.overrides[0].operation,
        PrefabOverrideOperation::Set { .. }
    ));
    assert!(matches!(
        snapshot.overrides[1].operation,
        PrefabOverrideOperation::Clear { .. }
    ));
    assert!(matches!(
        snapshot.overrides[2].operation,
        PrefabOverrideOperation::Insert { index: 1, .. }
    ));
    assert!(matches!(
        snapshot.overrides[3].operation,
        PrefabOverrideOperation::Remove { index: 2, .. }
    ));
    assert!(matches!(
        snapshot.overrides[4].operation,
        PrefabOverrideOperation::Move { from: 3, to: 4, .. }
    ));
}

#[test]
fn vnext_rpc_scalar_list_map_enum_entity_component_and_override_edits_round_trip() {
    let harness = RpcHarness::new();
    let client = &harness.client;
    let mut revision = harness.open(BASELINE_SOURCE).status.revision;
    revision = round_trip_scalar_edit(client, revision);
    revision = round_trip_list_edits(client, revision);
    revision = round_trip_map_edits(client, revision);
    revision = round_trip_variant_edit(client, revision);
    revision = round_trip_entity_and_component_edits(client, revision);
    round_trip_instance_edits(client, revision);
    round_trip_override_edits(&harness);
}

fn round_trip_scalar_edit(client: &project_capnp::project_host::Client, revision: u64) -> u64 {
    let mut revision = revision;
    let scalar_target = component_target::<VNextHarnessComponent>(
        "fixture-root",
        vec![ReflectedPathSegment::Field("scalar".to_owned())],
    );
    let result = apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::SetValue {
            target: scalar_target,
            value: envelope::<f32>("4.5"),
        },
    );
    let snapshot = snapshot_from(result);
    revision = snapshot.revision;
    assert_harness_scalar(&snapshot, 4.5);
    revision
}

fn round_trip_list_edits(client: &project_capnp::project_host::Client, revision: u64) -> u64 {
    let mut revision = revision;
    let list_target = component_target::<VNextHarnessComponent>(
        "fixture-root",
        vec![ReflectedPathSegment::Field("items".to_owned())],
    );
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::ListInsert {
            target: list_target.clone(),
            index: 1,
            value: envelope::<f32>("9.0"),
        },
    ))
    .revision;
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::ListMove {
            target: list_target.clone(),
            from: 1,
            to: 0,
        },
    ))
    .revision;
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::ListRemove {
            target: list_target,
            index: 0,
        },
    ))
    .revision;
    revision
}

fn round_trip_map_edits(client: &project_capnp::project_host::Client, revision: u64) -> u64 {
    let mut revision = revision;
    let map_target = component_target::<VNextHarnessComponent>(
        "fixture-root",
        vec![ReflectedPathSegment::Field("values".to_owned())],
    );
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::MapInsert {
            target: map_target.clone(),
            key: envelope::<String>(r#""added""#),
            value: envelope::<f32>("7.0"),
        },
    ))
    .revision;
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::MapRemove {
            target: map_target,
            key: envelope::<String>(r#""added""#),
        },
    ))
    .revision;
    revision
}

fn round_trip_variant_edit(client: &project_capnp::project_host::Client, revision: u64) -> u64 {
    let mut revision = revision;
    let mode_target = component_target::<VNextHarnessComponent>(
        "fixture-root",
        vec![ReflectedPathSegment::Field("mode".to_owned())],
    );
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::SetVariant {
            target: mode_target,
            variant_name: "Beta".to_owned(),
            value: None,
        },
    ))
    .revision;
    revision
}

fn round_trip_entity_and_component_edits(
    client: &project_capnp::project_host::Client,
    revision: u64,
) -> u64 {
    let mut revision = revision;
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::AddEntity {
            alias: "child".to_owned(),
            parent_alias: Some("fixture-root".to_owned()),
        },
    ))
    .revision;
    let snapshot = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::AddComponent {
            entity_alias: "child".to_owned(),
            component_type_path: type_path::<az_transform::Transform>(),
            initial_value: None,
        },
    ));
    revision = snapshot.revision;
    assert!(
        snapshot
            .hierarchy
            .iter()
            .any(|edge| edge.child_alias == "child"
                && edge.parent_alias.as_deref() == Some("fixture-root"))
    );
    component::<az_transform::Transform>(&snapshot, "child");
    let snapshot = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::ReparentEntity {
            alias: "child".to_owned(),
            parent_alias: None,
        },
    ));
    revision = snapshot.revision;
    assert!(
        snapshot
            .hierarchy
            .iter()
            .any(|edge| { edge.child_alias == "child" && edge.parent_alias.is_none() })
    );
    revision = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::RemoveComponent {
            entity_alias: "child".to_owned(),
            component_type_path: type_path::<az_transform::Transform>(),
        },
    ))
    .revision;
    let snapshot = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::RemoveEntity {
            alias: "child".to_owned(),
        },
    ));
    assert!(
        snapshot
            .entities
            .iter()
            .all(|entity| entity.alias != "child")
    );
    revision = snapshot.revision;
    revision
}

fn round_trip_instance_edits(client: &project_capnp::project_host::Client, revision: u64) {
    let mut revision = revision;
    let snapshot = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::AddInstance {
            alias: "runtime-instance".to_owned(),
            source_asset: "prefabs/base.prefab.ron".to_owned(),
            parent_entity_alias: Some("fixture-root".to_owned()),
        },
    ));
    revision = snapshot.revision;
    assert!(snapshot.instances.iter().any(|instance| {
        instance.alias == "runtime-instance"
            && instance.parent_entity_alias.as_deref() == Some("fixture-root")
    }));
    let snapshot = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::ReparentInstance {
            alias: "runtime-instance".to_owned(),
            parent_entity_alias: None,
        },
    ));
    revision = snapshot.revision;
    assert!(snapshot.instances.iter().any(|instance| {
        instance.alias == "runtime-instance" && instance.parent_entity_alias.is_none()
    }));
    let snapshot = snapshot_from(apply_edit(
        client,
        BASELINE_SOURCE,
        revision,
        &PrefabEditCommand::RemoveInstance {
            alias: "runtime-instance".to_owned(),
        },
    ));
    assert!(
        snapshot
            .instances
            .iter()
            .all(|instance| instance.alias != "runtime-instance")
    );
}

fn round_trip_override_edits(harness: &RpcHarness) {
    let nested = harness.open(NESTED_SOURCE);
    let override_target = nested.snapshot.expect("nested snapshot").overrides[0]
        .operation
        .target()
        .clone();
    let cleared = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        nested.status.revision,
        &PrefabEditCommand::RemoveOverride {
            target: override_target.clone(),
        },
    ));
    assert_eq!(cleared.overrides.len(), 4);
    let restored = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        cleared.revision,
        &PrefabEditCommand::SetOverride {
            target: override_target,
            value: envelope::<f32>("3.0"),
        },
    ));
    assert_eq!(restored.overrides.len(), 5);
}

#[test]
fn vnext_rpc_full_override_operation_edits_survive_snapshot_round_trips() {
    let harness = RpcHarness::new();
    let opened = harness.open(NESTED_SOURCE);
    let initial = opened.snapshot.expect("nested snapshot");
    let scalar_target = initial.overrides[0].operation.target().clone();
    let list_target = initial.overrides[2].operation.target().clone();
    let mut revision = opened.status.revision;

    let snapshot = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        revision,
        &PrefabEditCommand::ClearOverride {
            target: scalar_target.clone(),
        },
    ));
    revision = snapshot.revision;
    assert!(matches!(
        snapshot.overrides[0].operation,
        PrefabOverrideOperation::Clear { .. }
    ));

    let snapshot = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        revision,
        &PrefabEditCommand::SetOverride {
            target: scalar_target,
            value: envelope::<f32>("3.0"),
        },
    ));
    revision = snapshot.revision;
    assert!(matches!(
        snapshot.overrides[0].operation,
        PrefabOverrideOperation::Set { .. }
    ));

    let snapshot = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        revision,
        &PrefabEditCommand::InsertOverride {
            target: list_target.clone(),
            index: 8,
            value: envelope::<f32>("5.0"),
        },
    ));
    revision = snapshot.revision;
    assert!(matches!(
        snapshot.overrides[2].operation,
        PrefabOverrideOperation::Insert { index: 8, .. }
    ));

    let snapshot = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        revision,
        &PrefabEditCommand::RemoveOverrideItem {
            target: list_target.clone(),
            index: 9,
        },
    ));
    revision = snapshot.revision;
    assert!(matches!(
        snapshot.overrides[2].operation,
        PrefabOverrideOperation::Remove { index: 9, .. }
    ));

    let snapshot = snapshot_from(apply_edit(
        &harness.client,
        NESTED_SOURCE,
        revision,
        &PrefabEditCommand::MoveOverride {
            target: list_target,
            from: 10,
            to: 11,
        },
    ));
    assert!(matches!(
        snapshot.overrides[2].operation,
        PrefabOverrideOperation::Move {
            from: 10,
            to: 11,
            ..
        }
    ));
}

#[test]
fn vnext_rpc_undo_redo_lifecycle_restores_prior_and_next_snapshots() {
    let harness = RpcHarness::new();
    let opened = harness.open(BASELINE_SOURCE);
    let target = component_target::<VNextHarnessComponent>(
        "fixture-root",
        vec![ReflectedPathSegment::Field("scalar".to_owned())],
    );
    let edited = snapshot_from(apply_edit(
        &harness.client,
        BASELINE_SOURCE,
        opened.status.revision,
        &PrefabEditCommand::SetValue {
            target,
            value: envelope::<f32>("8.0"),
        },
    ));
    assert_harness_scalar(&edited, 8.0);

    let undone = lifecycle(
        &harness.client,
        BASELINE_SOURCE,
        SourceSessionCommand::Undo,
        edited.revision,
    );
    assert_eq!(undone.status.undo_depth, 0);
    assert_eq!(undone.status.redo_depth, 1);
    let undone_snapshot = undone.snapshot.expect("undo snapshot");
    assert_harness_scalar(&undone_snapshot, 1.0);

    let redone = lifecycle(
        &harness.client,
        BASELINE_SOURCE,
        SourceSessionCommand::Redo,
        undone.status.revision,
    );
    let redone_snapshot = redone.snapshot.expect("redo snapshot");
    assert_harness_scalar(&redone_snapshot, 8.0);
    let status = lifecycle(
        &harness.client,
        BASELINE_SOURCE,
        SourceSessionCommand::Status,
        redone.status.revision,
    );
    assert!(status.status.open);
    assert!(!status.status.dirty);
    let saved = lifecycle(
        &harness.client,
        BASELINE_SOURCE,
        SourceSessionCommand::Save,
        status.status.revision,
    );
    assert!(saved.diagnostics.is_empty());
    let recovery = lifecycle(
        &harness.client,
        BASELINE_SOURCE,
        SourceSessionCommand::SaveRecovery,
        saved.status.revision,
    );
    assert!(recovery.diagnostics.is_empty());
    let closed = lifecycle(
        &harness.client,
        BASELINE_SOURCE,
        SourceSessionCommand::Close,
        recovery.status.revision,
    );
    assert!(!closed.status.open);
}

#[test]
fn vnext_rpc_validation_reports_camera_and_spot_light_named_paths() {
    let harness = RpcHarness::new();
    let opened = harness.open(INVALID_SOURCE);
    assert!(
        opened.diagnostics.is_empty(),
        "opening does not reject inspectable invalid source"
    );
    let diagnostics = diagnostics(&harness.client, INVALID_SOURCE);
    let camera = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "camera.near_before_far")
        .expect("Camera cross-field diagnostic");
    let camera_target = camera.target.as_ref().expect("Camera target");
    assert_eq!(
        camera_target.path.component_type_path,
        type_path::<az_render::Camera>()
    );
    assert_eq!(
        camera_target.path.segments,
        [ReflectedPathSegment::Field("near".to_owned())]
    );
    let spot = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "spot_light.inner_before_outer")
        .expect("SpotLight cross-field diagnostic");
    let spot_target = spot.target.as_ref().expect("SpotLight target");
    assert_eq!(
        spot_target.path.component_type_path,
        type_path::<az_render::SpotLight>()
    );
    assert_eq!(
        spot_target.path.segments,
        [ReflectedPathSegment::Field(
            "inner_angle_degrees".to_owned()
        )]
    );
}

#[test]
fn vnext_rpc_typed_action_executes_host_policy_and_returns_changed_path() {
    let harness = RpcHarness::new();
    let opened = harness.open(BASELINE_SOURCE);
    let target = component_target::<VNextHarnessComponent>("fixture-root", Vec::new());
    let result = invoke_action(
        &harness.client,
        BASELINE_SOURCE,
        opened.status.revision,
        &target,
    );
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    assert_eq!(
        result.changed_paths,
        [ReflectedPath {
            component_type_path: type_path::<VNextHarnessComponent>(),
            segments: vec![ReflectedPathSegment::Field("scalar".to_owned())],
        }]
    );
    let snapshot = result.snapshot.expect("action snapshot");
    assert_harness_scalar(&snapshot, 0.0);
}

/// A `SetVariant` command carrying no payload — the only form the inspector's
/// variant dropdown sends — names a variant and nothing else, so the host is
/// what reads the target variant's declared shape and writes that shape's
/// empty value. It used to write a unit payload for every shape, so switching
/// an enum to a struct-shaped variant from the editor silently put `Named`
/// into the document: the one spelling ticket 046 proved the producer rejects.
///
/// The emission table below is 046's, verified there against the real
/// producer. A unit variant is spelled bare, a variant declaring no field
/// takes an empty body, and a struct-shaped variant retaining nothing takes an
/// empty body too. A tuple variant that declares fields can never be
/// sparse-empty — `validate_sparse_enum` demands the exact declared count — so
/// the host authors each declared field type's registered reflected default.
///
/// Every payload here is the host's own: the snapshot envelopes come back
/// through the real capnp client, encoded by the real `PrefabCodec`.
#[test]
fn vnext_rpc_set_variant_without_a_value_applies_the_target_variant_shape() {
    let harness = RpcHarness::new();
    let mut revision = harness.open(VARIANT_SOURCE).status.revision;
    let mode = component_target::<VariantShapeComponent>(
        "variant-root",
        vec![ReflectedPathSegment::Field("mode".to_owned())],
    );
    assert_empty_variant_payloads_take_the_declared_shape(&harness, &mode, &mut revision);
    assert_explicit_variant_payloads_apply_verbatim(&harness, &mode, &mut revision);
    assert_mismatched_and_undeclared_variants_are_refused(&harness, &mode, revision);
    assert_variant_emission_reparses(&harness, &mode, &mut revision);
}

fn variant_payload(snapshot: &PrefabSourceSnapshot) -> String {
    String::from_utf8(
        component::<VariantShapeComponent>(snapshot, "variant-root")
            .sparse_value
            .payload
            .clone(),
    )
    .expect("utf-8 sparse payload")
}

fn switch_variant(
    harness: &RpcHarness,
    mode: &PrefabValueTarget,
    revision: &mut u64,
    variant: &str,
    value: Option<ReflectedValueEnvelope>,
) -> String {
    let snapshot = snapshot_from(apply_edit(
        &harness.client,
        VARIANT_SOURCE,
        *revision,
        &PrefabEditCommand::SetVariant {
            target: mode.clone(),
            variant_name: variant.to_owned(),
            value,
        },
    ));
    *revision = snapshot.revision;
    variant_payload(&snapshot)
}

fn assert_empty_variant_payloads_take_the_declared_shape(
    harness: &RpcHarness,
    mode: &PrefabValueTarget,
    revision: &mut u64,
) {
    // The fix: a struct-shaped variant receives its declared shape's empty
    // named-field set, not a unit payload.
    assert_eq!(
        switch_variant(harness, mode, revision, "Named", None),
        "(mode:Named())",
        "a struct-shaped variant takes an empty body",
    );
    // A variant declaring no field keeps the empty body it always spelled...
    assert_eq!(
        switch_variant(harness, mode, revision, "Fieldless", None),
        "(mode:Fieldless())"
    );
    // ...and a unit variant keeps the bare spelling, unchanged by this ticket.
    assert_eq!(
        switch_variant(harness, mode, revision, "Marker", None),
        "(mode:Marker)"
    );
    // A tuple variant carrying fields cannot omit them, so each declared field
    // arrives as its own type's reflected default — `f32` and `bool` differ,
    // so this is per-field, not one blanket value.
    assert_eq!(
        switch_variant(harness, mode, revision, "Single", None),
        "(mode:Single(0.0))"
    );
    assert_eq!(
        switch_variant(harness, mode, revision, "Pair", None),
        "(mode:Pair(0.0,false))",
    );
}

fn assert_explicit_variant_payloads_apply_verbatim(
    harness: &RpcHarness,
    mode: &PrefabValueTarget,
    revision: &mut u64,
) {
    // Negative control: a command that does carry a payload still applies that
    // payload verbatim, for every shape.
    assert_eq!(
        switch_variant(
            harness,
            mode,
            revision,
            "Named",
            Some(envelope::<VariantShapeMode>("Named(alpha:1.5,beta:true)")),
        ),
        "(mode:Named(alpha:1.5,beta:true))",
    );
    assert_eq!(
        switch_variant(
            harness,
            mode,
            revision,
            "Pair",
            Some(envelope::<VariantShapeMode>("Pair(2.5,true)")),
        ),
        "(mode:Pair(2.5,true))",
    );
}

fn assert_mismatched_and_undeclared_variants_are_refused(
    harness: &RpcHarness,
    mode: &PrefabValueTarget,
    revision: u64,
) {
    // Negative control: a payload naming a different variant is still refused.
    let mismatched = apply_edit(
        &harness.client,
        VARIANT_SOURCE,
        revision,
        &PrefabEditCommand::SetVariant {
            target: mode.clone(),
            variant_name: "Marker".to_owned(),
            value: Some(envelope::<VariantShapeMode>("Single(1.0)")),
        },
    );
    assert!(
        mismatched
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("expected `Marker`")),
        "{:?}",
        mismatched.diagnostics,
    );
    // Negative control: a variant the enum does not declare is refused rather
    // than written, and the document keeps the variant it had.
    let unknown = apply_edit(
        &harness.client,
        VARIANT_SOURCE,
        revision,
        &PrefabEditCommand::SetVariant {
            target: mode.clone(),
            variant_name: "Absent".to_owned(),
            value: None,
        },
    );
    assert!(
        unknown
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("has no variant `Absent`")),
        "{:?}",
        unknown.diagnostics,
    );
}

fn assert_variant_emission_reparses(
    harness: &RpcHarness,
    mode: &PrefabValueTarget,
    revision: &mut u64,
) {
    // The producer parses its own emission: switch back to the struct-shaped
    // variant, then save, close, and reopen through the real codec.
    assert_eq!(
        switch_variant(harness, mode, revision, "Named", None),
        "(mode:Named())"
    );
    let saved = lifecycle(
        &harness.client,
        VARIANT_SOURCE,
        SourceSessionCommand::Save,
        *revision,
    );
    assert!(saved.diagnostics.is_empty(), "{:?}", saved.diagnostics);
    let closed = lifecycle(
        &harness.client,
        VARIANT_SOURCE,
        SourceSessionCommand::Close,
        saved.status.revision,
    );
    assert!(!closed.status.open);
    let reopened = harness.open(VARIANT_SOURCE);
    assert!(
        reopened.diagnostics.is_empty(),
        "{:?}",
        reopened.diagnostics
    );
    assert_eq!(
        variant_payload(&reopened.snapshot.expect("reopened variant snapshot")),
        "(mode:Named())",
    );
}

#[test]
#[ignore = "Phase 4b: GPUI renderer/layout parity for multiline text, asset/object references, and mixed multi-selection"]
fn vnext_parity_phase4b_multiline_asset_object_refs_and_mixed_selection_renderer_placeholder() {
    unreachable!("Phase 4b owns editor renderer/layout parity")
}

#[test]
#[ignore = "Phase 4b: editor consumes static/dynamic visibility, read-only, and Add Component policy"]
fn vnext_parity_phase4b_visibility_read_only_and_add_component_ui_placeholder() {
    unreachable!("Phase 4b owns inspector policy presentation")
}

#[test]
#[ignore = "Phase 4b: GameData tables and graph-port renderers remain on their independent RPCs until UI cutover"]
fn vnext_parity_phase4b_gamedata_and_graph_ports_placeholder() {
    unreachable!("Phase 4b owns non-Prefab inspector consumers")
}
