use std::{
    any::TypeId,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use az_core::{
    EditorFieldAttributes, EditorFieldConstraints, EditorNumericRange, EditorTypeAttributes,
    EditorWidget,
};
use az_editor_inspector::{
    ReflectedAddComponent, ReflectedInspectionChild, ReflectedInspectionField,
    ReflectedInspectionInput, ReflectedInspectionModel, ReflectedOverrideOperation,
    ReflectedProjectionError, ReflectedScalar, ReflectedValue, ReflectedValueNode,
    ReflectedVariantSelection, WidgetFamily, decode_reflected_envelope,
};
use az_gem_contract::{
    Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
    GemTargetRole, ProductActivation, declare_caps,
};
use az_prefab::{Prefab, PrefabCodec, PrefabType, ReflectPrefab};
use az_project_host::{Composition, ProjectHost, ProjectHostRpc};
use az_proto_core::{Capability, CapabilityGrantSet, ServiceId, ServiceRole};
use az_proto_project::{
    PROJECT_DOCUMENT_READ_PERMISSION, PROJECT_DOCUMENT_WRITE_PERMISSION, PROJECT_EDIT_PERMISSION,
    PROJECT_HOST_AUDIENCE, PROJECT_SCHEMA_PERMISSION, project_capnp,
    vnext::{
        PrefabComponentSnapshot, PrefabDiagnostic, PrefabEditCommand, PrefabRpcResult,
        PrefabSourceSnapshot, ReflectedPath, ReflectedPathSegment, ReflectedTypeDescriptor,
        ReflectedValueEncoding, ReflectedValueEnvelope, SourceSessionCommand, SourceSessionResult,
        TypeRegistrySnapshot,
    },
};
use bevy_ecs::{component::Component, reflect::ReflectComponent};
use bevy_reflect::{
    Reflect, TypePath, TypeRegistry,
    enums::{DynamicEnum, DynamicVariant},
    std_traits::ReflectDefault,
    structs::DynamicStruct,
    tuple::DynamicTuple,
};
use serde::Deserialize;

const BASELINE: &str = "component-baseline.prefab.ron";
const DEFAULTS: &str = "reflected-defaults.prefab.ron";
const VALIDATION: &str = "validation.prefab.ron";
const NESTED_OVERRIDE: &str = "nested-override.prefab.ron";
const TOKEN_HASH: [u8; 2] = [0x70, 0x48];

#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Default)]
struct NestedItem {
    #[reflect(@EditorFieldAttributes::new("Name", EditorWidget::Default))]
    name: String,
    #[reflect(@EditorFieldAttributes::new("Weight", EditorWidget::Number).with_range(
        EditorNumericRange {
            step: Some("0.25".to_owned()),
            ..EditorNumericRange::default()
        },
    ))]
    weight: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect)]
#[reflect(Default)]
enum NestedMode {
    #[default]
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Nested Collections").in_group("Phase 0"))]
#[prefab(tag = "NestedCollections", version = 1)]
struct NestedCollections {
    #[reflect(@EditorFieldAttributes::new("Items", EditorWidget::Default))]
    items: Vec<NestedItem>,
    #[reflect(@EditorFieldAttributes::new(
        "Weights By Name",
        EditorWidget::Default,
    ))]
    weights_by_name: BTreeMap<String, f32>,
    #[reflect(@EditorFieldAttributes::new(
        "Mode",
        EditorWidget::Dropdown {
            choices: vec!["Automatic".to_owned(), "Manual".to_owned()],
        },
    ))]
    mode: NestedMode,
}

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Constraint Projection").in_group("Phase 0"))]
#[prefab(tag = "ConstraintProjection", version = 1)]
struct ConstraintProjection {
    #[reflect(@EditorFieldAttributes::new("Text", EditorWidget::Default).with_constraints(
        EditorFieldConstraints {
            minimum_length: Some(1),
            maximum_length: Some(8),
            allowed_strings: vec!["first".to_owned()],
            allowed_variants: Vec::new(),
        },
    ))]
    text: String,
    #[reflect(@EditorFieldAttributes::new(
        "Mode",
        EditorWidget::Dropdown {
            choices: vec!["Automatic".to_owned(), "Manual".to_owned()],
        },
    ).with_constraints(EditorFieldConstraints {
        allowed_variants: vec!["Automatic".to_owned()],
        ..EditorFieldConstraints::default()
    }))]
    mode: NestedMode,
}

#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Typed Map Keys").in_group("Phase 0"))]
#[prefab(tag = "TypedMapKeys", version = 1)]
struct TypedMapKeys {
    #[reflect(@EditorFieldAttributes::new("Values", EditorWidget::Default))]
    values: BTreeMap<u32, f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Reflect)]
#[reflect(Default)]
struct FixtureObjectReference {
    entity_alias: String,
}

#[derive(Debug, Clone, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[reflect(@EditorTypeAttributes::labeled("Inspector Behaviors")
    .in_group("Phase 0")
    .with_action("adr0022.reset"))]
#[prefab(tag = "InspectorBehaviors", version = 1)]
struct InspectorBehaviors {
    #[reflect(@EditorFieldAttributes::new("Scalar", EditorWidget::Number))]
    scalar: f32,
    #[reflect(@EditorFieldAttributes::new(
        "Notes",
        EditorWidget::Multiline { rows: Some(4) },
    ))]
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
    #[reflect(@EditorFieldAttributes::new(
        "Asset",
        EditorWidget::AssetPicker {
            asset_type_path: "mesh".to_owned(),
        },
    ))]
    asset: String,
    #[reflect(@EditorFieldAttributes::new(
        "Object",
        EditorWidget::ObjectPicker {
            object_type_path: "entity".to_owned(),
        },
    ))]
    object: FixtureObjectReference,
}

impl Default for InspectorBehaviors {
    fn default() -> Self {
        Self {
            scalar: 1.0,
            notes: "line one\nline two".to_owned(),
            locked: true,
            internal: false,
            asset: String::new(),
            object: FixtureObjectReference::default(),
        }
    }
}

declare_caps!(ParityCaps:);

/// Stands in for a gem's runtime contribution: the fixture types this parity
/// suite authors against, registered through the ordinary registrar.
struct Parity;

impl Contribution for Parity {
    type Caps = ParityCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.editor-inspector-tests"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, ParityCaps>) {
        ctx.registrar::<PrefabType>().register_many([
            PrefabType::of::<NestedItem>(),
            PrefabType::of::<NestedMode>(),
            PrefabType::of::<Vec<NestedItem>>(),
            PrefabType::of::<BTreeMap<String, f32>>(),
            PrefabType::of::<NestedCollections>(),
            PrefabType::of::<ConstraintProjection>(),
            PrefabType::of::<BTreeMap<u32, f32>>(),
            PrefabType::of::<TypedMapKeys>(),
            PrefabType::of::<FixtureObjectReference>(),
            PrefabType::of::<InspectorBehaviors>(),
        ]);
    }
}

fn parity_composition() -> Composition {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(Parity, ProductActivation::default())
        .expect("an empty capability floor composes");
    Composition::new(composer).expect("inspector parity composition is valid and ready")
}

struct RpcHarness {
    client: project_capnp::project_host::Client,
    _rpc: Rc<ProjectHostRpc>,
    _composition: Composition,
    _temp: tempfile::TempDir,
}

impl RpcHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("typed parity source root");
        az_project::write_project_manifest(
            temp.path(),
            &az_project::ProjectManifest::new(
                "local.az_editor_inspector_parity",
                "Editor Inspector Parity",
                "0.1.0",
            ),
        )
        .expect("write parity project manifest");
        for source in [BASELINE, DEFAULTS, VALIDATION, NESTED_OVERRIDE] {
            fs::copy(typed_fixture_root().join(source), temp.path().join(source))
                .unwrap_or_else(|error| panic!("copy typed fixture {source}: {error}"));
        }
        let composition = parity_composition();
        let rpc = Rc::new(ProjectHostRpc::test_new_composed(
            ProjectHost::with_source_root(temp.path()),
            capability_grants(),
            &composition,
        ));
        assert_prefab_type_data(&rpc.registry().app_registry.read());
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

#[test]
fn typed_sources_round_trip_through_vnext_and_project_recursive_neutral_data() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let baseline = open_snapshot(&harness, BASELINE);
    let defaults = open_snapshot(&harness, DEFAULTS);
    assert_every_field_binds_to_its_own_component(&registry, &baseline, &defaults);

    let transform = model(&registry, &baseline, &defaults, "Transform", &[]);
    assert_eq!(
        transform.fields[0].widget.family,
        WidgetFamily::Vector { dimensions: 3 }
    );
    assert_eq!(transform.fields[1].widget.family, WidgetFamily::Quaternion);
    assert_eq!(
        transform.fields[2].value.default.value,
        Some(ReflectedValue::Tuple(vec![
            float("1.0"),
            float("1.0"),
            float("1.0"),
        ]))
    );

    let camera = model(&registry, &baseline, &defaults, "Camera", &[]);
    assert!(matches!(
        camera.fields[0].value.children.as_slice(),
        [ReflectedInspectionChild::Variant(variant)] if variant.name == "Perspective"
    ));

    let nested = model(&registry, &baseline, &defaults, "Nested Collections", &[]);
    assert!(matches!(
        nested.fields[0].value.children.as_slice(),
        [ReflectedInspectionChild::ListItem(_)]
    ));
    assert!(matches!(
        nested.fields[1].value.children.as_slice(),
        [ReflectedInspectionChild::MapEntry(_)]
    ));
    assert!(matches!(
        nested.fields[2].value.children.as_slice(),
        [ReflectedInspectionChild::Variant(variant)] if variant.name == "Automatic"
    ));

    let typed_map = model(&registry, &baseline, &defaults, "Typed Map Keys", &[]);
    let ReflectedInspectionChild::MapEntry(entry) = &typed_map.fields[0].value.children[0] else {
        panic!("typed map should project an entry");
    };
    assert!(matches!(
        entry.binding.remove(),
        PrefabEditCommand::MapRemove { key, .. }
            if key.type_path == <u32 as TypePath>::type_path()
                && key.encoding == ReflectedValueEncoding::TypedRon
    ));

    let behavior = model(&registry, &baseline, &defaults, "Inspector Behaviors", &[]);
    assert_inspector_behavior_projection(&behavior);
}

/// Every projected field of every Phase 0 fixture binds to its own component
/// under a single named-field path.
fn assert_every_field_binds_to_its_own_component(
    registry: &TypeRegistrySnapshot,
    baseline: &PrefabSourceSnapshot,
    defaults: &PrefabSourceSnapshot,
) {
    for label in [
        "Transform",
        "Mesh",
        "Material Assignment",
        "Camera",
        "Directional Light",
        "Point Light",
        "Nested Collections",
        "Typed Map Keys",
        "Inspector Behaviors",
    ] {
        let model = model(registry, baseline, defaults, label, &[]);
        assert_eq!(model.type_label, label);
        assert!(!model.fields.is_empty());
        assert!(model.fields.iter().all(|field| {
            field.value.binding.target.path.component_type_path == model.type_path
                && matches!(
                    field.value.binding.target.path.segments.as_slice(),
                    [ReflectedPathSegment::Field(name)] if name == &field.name
                )
        }));
    }
}

/// Read-only, hidden, asset and object fields keep the projection their editor
/// attributes declare.
fn assert_inspector_behavior_projection(behavior: &ReflectedInspectionModel) {
    assert!(
        behavior
            .fields
            .iter()
            .find(|field| field.name == "locked")
            .unwrap()
            .read_only
    );
    assert!(
        behavior
            .fields
            .iter()
            .find(|field| field.name == "internal")
            .unwrap()
            .hidden
    );
    assert!(matches!(
        behavior
            .fields
            .iter()
            .find(|field| field.name == "asset")
            .unwrap()
            .widget
            .family,
        WidgetFamily::Asset { .. }
    ));
    assert!(matches!(
        behavior
            .fields
            .iter()
            .find(|field| field.name == "object")
            .unwrap()
            .widget
            .family,
        WidgetFamily::Object { .. }
    ));
}

#[test]
fn explicit_none_survives_inspector_edit_save_and_reopen_without_default_some() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let opened = harness.open(BASELINE);
    let revision = opened.status.revision;
    let baseline = opened.snapshot.expect("baseline snapshot");
    let material = model(&registry, &baseline, &baseline, "Material Assignment", &[]);
    let field = material
        .fields
        .iter()
        .find(|field| field.name == "default")
        .expect("default material Option");
    assert!(matches!(
        field.value.children.as_slice(),
        [ReflectedInspectionChild::OptionalSome(_)]
    ));

    let edited = snapshot_from(apply_edit(
        &harness.client,
        BASELINE,
        revision,
        &field.value.binding.set_variant("None", None),
    ));
    let assert_explicit_none = |snapshot: &PrefabSourceSnapshot| {
        let material = model(&registry, snapshot, &baseline, "Material Assignment", &[]);
        let field = material
            .fields
            .iter()
            .find(|field| field.name == "default")
            .expect("default material Option");
        assert_eq!(
            field.value.current.authored,
            Some(ReflectedValue::Optional(None))
        );
        assert_eq!(
            field.value.current.effective,
            Some(ReflectedValue::Optional(None))
        );
        assert!(field.value.children.is_empty());
    };
    assert_explicit_none(&edited);

    let saved = lifecycle(
        &harness.client,
        BASELINE,
        SourceSessionCommand::Save,
        edited.revision,
    );
    assert!(saved.diagnostics.is_empty());
    let closed = lifecycle(
        &harness.client,
        BASELINE,
        SourceSessionCommand::Close,
        saved.status.revision,
    );
    assert!(!closed.status.open);
    let reopened = harness.open(BASELINE);
    assert!(reopened.diagnostics.is_empty());
    assert_explicit_none(&reopened.snapshot.expect("reopened snapshot"));
}

#[test]
fn validation_undo_redo_and_nested_override_survive_real_capnp_round_trip() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let validation = open_snapshot(&harness, VALIDATION);
    let diagnostics = prefab_diagnostics(&harness.client, VALIDATION);
    let camera = model(&registry, &validation, &validation, "Camera", &diagnostics);
    let near = camera
        .fields
        .iter()
        .find(|field| field.name == "near")
        .unwrap();
    assert!(!near.validation.is_valid());
    assert!(near.validation.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "camera.near_before_far"
            && diagnostic.target.as_ref().is_some_and(|target| {
                target.path.segments == [ReflectedPathSegment::Field("near".to_owned())]
            })
    }));

    let opened = harness.open(BASELINE);
    let edited = snapshot_from(apply_edit(
        &harness.client,
        BASELINE,
        opened.status.revision,
        &PrefabEditCommand::AddEntity {
            alias: "undo-child".to_owned(),
            parent_alias: Some("fixture-root".to_owned()),
        },
    ));
    assert!(
        edited
            .entities
            .iter()
            .any(|entity| entity.alias == "undo-child")
    );
    let undone = lifecycle(
        &harness.client,
        BASELINE,
        SourceSessionCommand::Undo,
        edited.revision,
    );
    assert!(undone.snapshot.as_ref().is_some_and(|snapshot| {
        snapshot
            .entities
            .iter()
            .all(|entity| entity.alias != "undo-child")
    }));
    let redone = lifecycle(
        &harness.client,
        BASELINE,
        SourceSessionCommand::Redo,
        undone.status.revision,
    );
    assert!(redone.snapshot.as_ref().is_some_and(|snapshot| {
        snapshot
            .entities
            .iter()
            .any(|entity| entity.alias == "undo-child")
    }));

    let nested = open_snapshot(&harness, NESTED_OVERRIDE);
    assert_eq!(nested.instances[0].alias, "door-instance");
    assert_eq!(nested.overrides.len(), 2);
    let set = ReflectedOverrideOperation::project(&nested.overrides[0]);
    let clear = ReflectedOverrideOperation::project(&nested.overrides[1]);
    assert_eq!(
        nested.overrides[0].operation.target().instance_alias_chain,
        ["door-instance"]
    );
    assert_eq!(
        nested.overrides[0].operation.target().path.segments,
        [ReflectedPathSegment::Field("near".to_owned())]
    );
    assert!(matches!(clear, ReflectedOverrideOperation::Clear { .. }));
    let after_edit = snapshot_from(apply_edit(
        &harness.client,
        NESTED_OVERRIDE,
        nested.revision,
        &set.edit_command(),
    ));
    assert!(after_edit.overrides.iter().any(|snapshot| {
        matches!(
            ReflectedOverrideOperation::project(snapshot),
            ReflectedOverrideOperation::Clear { ref target }
                if target.path.segments == [ReflectedPathSegment::Field("far".to_owned())]
        )
    }));
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenProjection {
    case: String,
    schema: String,
    type_label: String,
    category: Option<String>,
    icon: Option<String>,
    fields: Vec<GoldenField>,
    add_component: Option<GoldenAddComponent>,
    behaviors: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenField {
    name: String,
    label: String,
    schema_type: String,
    widget: String,
    default: Option<String>,
    constraints: GoldenConstraints,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
struct GoldenConstraints {
    min: Option<f64>,
    max: Option<f64>,
    min_length: Option<u32>,
    max_length: Option<u32>,
    allowed_strings: Vec<String>,
    allowed_variants: Vec<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GoldenAddComponent {
    editor_export: bool,
    runtime_export: bool,
    provides: Vec<String>,
    requires: Vec<String>,
    incompatible: Vec<String>,
    default_available: bool,
}

#[test]
fn normalized_vnext_projection_matches_phase_zero_user_facing_fields() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let baseline = open_snapshot(&harness, BASELINE);
    let defaults = open_snapshot(&harness, DEFAULTS);

    for (fixture_name, label) in [
        ("transform.inspector.ron", "Transform"),
        ("mesh.inspector.ron", "Mesh"),
        ("material-assignment.inspector.ron", "Material Assignment"),
        ("camera.inspector.ron", "Camera"),
        ("nested-collections.inspector.ron", "Nested Collections"),
    ] {
        let golden = golden(fixture_name);
        let actual = model(&registry, &baseline, &defaults, label, &[]);
        assert_eq!(
            actual.type_label, golden.type_label,
            "{} type label",
            golden.case
        );
        assert_eq!(actual.category, golden.category, "{} category", golden.case);
        assert_eq!(actual.icon, golden.icon, "{} icon", golden.case);
        assert_eq!(
            actual
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            golden
                .fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>(),
            "{} field set/order",
            golden.case,
        );
        assert!(!golden.schema.is_empty());
        assert!(!golden.behaviors.is_empty());

        for (actual, expected) in actual.fields.iter().zip(&golden.fields) {
            assert_golden_field(actual, expected, &golden.case);
        }

        if let Some(expected) = golden.add_component {
            assert_golden_add_component(&actual.add_component, &expected, &golden.case);
        }
    }
}

/// One projected field against its Phase 0 golden: label, widget family,
/// constraints, and default semantics.
fn assert_golden_field(actual: &ReflectedInspectionField, expected: &GoldenField, case: &str) {
    assert_eq!(
        actual.label, expected.label,
        "{case}.{} label",
        expected.name
    );
    assert_eq!(
        widget_family(&actual.widget.family),
        golden_widget_family(expected),
        "{case}.{} widget family (raw old widget `{}`; raw old schema `{}`)",
        expected.name,
        expected.widget,
        expected.schema_type,
    );
    assert_constraints(actual, expected, case);

    let expected_default = expected
        .default
        .as_deref()
        .map(|value| normalize_golden_default(expected, value));
    let actual_default = actual
        .value
        .default
        .value
        .as_ref()
        .map(normalize_reflected_default);
    assert_eq!(
        actual_default, expected_default,
        "{case}.{} default semantics",
        expected.name,
    );
}

/// The Add Component facts a projected model must agree with.
fn assert_golden_add_component(
    actual: &ReflectedAddComponent,
    expected: &GoldenAddComponent,
    case: &str,
) {
    assert_eq!(actual.editor_export, expected.editor_export);
    assert_eq!(actual.runtime_export, expected.runtime_export);
    assert_eq!(actual.default_available, expected.default_available);
    let az_editor_inspector::AddComponentCapabilities::Projected {
        provides,
        requires,
        incompatible,
    } = &actual.capabilities
    else {
        panic!("{case} Add Component capabilities were not projected");
    };
    assert_eq!(
        capability_set(provides),
        capability_set(&expected.provides),
        "{case} provides"
    );
    assert_eq!(
        capability_set(requires),
        capability_set(&expected.requires),
        "{case} requires"
    );
    assert_eq!(
        capability_set(incompatible),
        capability_set(&expected.incompatible),
        "{case} incompatible"
    );
}

#[test]
fn asset_object_reference_and_command_data_are_typed_without_legacy_ids() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let baseline = open_snapshot(&harness, BASELINE);
    let defaults = open_snapshot(&harness, DEFAULTS);
    let asset_golden = golden("asset-handle.inspector.ron");
    let mesh = model(&registry, &baseline, &defaults, "Mesh", &[]);
    let field = mesh
        .fields
        .iter()
        .find(|field| field.name == "mesh")
        .unwrap();
    assert_eq!(
        widget_family(&field.widget.family),
        golden_widget_family(&asset_golden.fields[0])
    );
    assert!(matches!(
        field.value.current.authored,
        Some(ReflectedValue::Scalar(ReflectedScalar::String(ref value)))
            if value == "meshes/fixture.azmesh"
    ));
    assert!(
        matches!(
            field.value.default.value,
            Some(ReflectedValue::Scalar(ReflectedScalar::String(ref value))) if value.is_empty()
        ),
        "ReflectDefault must project the empty AssetPathBuf"
    );

    let behavior = model(&registry, &baseline, &defaults, "Inspector Behaviors", &[]);
    let object = behavior
        .fields
        .iter()
        .find(|field| field.name == "object")
        .unwrap();
    assert!(matches!(object.widget.family, WidgetFamily::Object { .. }));
    assert!(matches!(
        object.value.current.authored,
        Some(ReflectedValue::Struct(ref fields))
            if fields.iter().any(|(name, value)| {
                name == "entity_alias"
                    && value == &ReflectedValue::Scalar(ReflectedScalar::String("fixture-root".to_owned()))
            })
    ));

    let scalar = behavior
        .fields
        .iter()
        .find(|field| field.name == "scalar")
        .unwrap();
    let command = scalar.value.binding.set_value(typed_envelope::<f32>("2.0"));
    assert!(matches!(
        command,
        PrefabEditCommand::SetValue { target, value }
            if target.path.segments == [ReflectedPathSegment::Field("scalar".to_owned())]
                && value.type_path == <f32 as TypePath>::type_path()
    ));
}

#[test]
fn add_component_applicability_matches_the_phase_zero_golden() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let baseline = open_snapshot(&harness, BASELINE);
    let defaults = open_snapshot(&harness, DEFAULTS);
    let golden = golden("add-component.inspector.ron");
    let expected = golden.add_component.expect("Add Component golden facts");
    let transform = model(&registry, &baseline, &defaults, "Transform", &[]);

    let az_editor_inspector::AddComponentCapabilities::Projected {
        provides,
        requires,
        incompatible,
    } = transform.add_component.capabilities
    else {
        panic!("Add Component capabilities were not projected");
    };
    assert_eq!(
        capability_set(&provides),
        capability_set(&expected.provides)
    );
    assert_eq!(
        capability_set(&requires),
        capability_set(&expected.requires)
    );
    assert_eq!(
        capability_set(&incompatible),
        capability_set(&expected.incompatible)
    );
    assert_eq!(
        transform.add_component.editor_export,
        expected.editor_export
    );
    assert_eq!(
        transform.add_component.runtime_export,
        expected.runtime_export
    );
    assert_eq!(
        transform.add_component.default_available,
        expected.default_available
    );
}

#[test]
fn reflected_field_constraints_compare_nonempty_legacy_semantics() {
    let harness = RpcHarness::new();
    let registry = registry_snapshot(&harness.client);
    let baseline = open_snapshot(&harness, BASELINE);
    let defaults = open_snapshot(&harness, DEFAULTS);
    let constrained = model(
        &registry,
        &baseline,
        &defaults,
        "Constraint Projection",
        &[],
    );

    let expected = [
        GoldenField {
            name: "text".to_owned(),
            label: "Text".to_owned(),
            schema_type: "core.string".to_owned(),
            widget: "default".to_owned(),
            default: Some("string:".to_owned()),
            constraints: GoldenConstraints {
                min_length: Some(1),
                max_length: Some(8),
                allowed_strings: vec!["first".to_owned()],
                ..GoldenConstraints::default()
            },
        },
        GoldenField {
            name: "mode".to_owned(),
            label: "Mode".to_owned(),
            schema_type: "azoth.phase0.NestedMode".to_owned(),
            widget: "dropdown".to_owned(),
            default: Some("variant:21()".to_owned()),
            constraints: GoldenConstraints {
                allowed_variants: vec![21],
                ..GoldenConstraints::default()
            },
        },
    ];
    for expected in &expected {
        let actual = constrained
            .fields
            .iter()
            .find(|field| field.name == expected.name)
            .expect("constrained reflected field");
        assert_constraints(actual, expected, "constraint_projection");
    }
}

#[test]
fn every_matrix_id_has_an_honest_data_status() {
    const DATA_PASS: &[&str] = &[
        "scalar_editing",
        "slider_range",
        "color_vector",
        "enum_variant",
        "nested_struct",
        "list_editing",
        "map_editing",
        "typed_map_keys",
        "undo_redo",
        "visibility_read_only",
        "validation",
        "asset_object_refs",
        "add_component",
    ];
    const PHASE_4B2: &[&str] = &[
        "multiline_text",
        "mixed_selection",
        "actions",
        "gamedata",
        "graph_ports",
    ];
    let statuses = DATA_PASS
        .iter()
        .chain(PHASE_4B2)
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let matrix = az_proto_project::vnext::INSPECTOR_PARITY_MATRIX
        .iter()
        .map(|case| case.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(statuses, matrix);
    assert_eq!(DATA_PASS.len(), 13);
    assert_eq!(PHASE_4B2.len(), 5);
}

#[test]
#[ignore = "4b-2: multiline GPUI rendering is outside the neutral data gate"]
fn phase_4b2_multiline_renderer_parity() {}

#[test]
#[ignore = "4b-2: mixed-selection interaction state belongs to the GPUI adapter"]
fn phase_4b2_mixed_selection_parity() {}

#[test]
#[ignore = "4b-2: typed action button rendering/invocation wiring belongs to the GPUI adapter"]
fn phase_4b2_actions_ui_parity() {}

#[test]
#[ignore = "4b-2: GameData is a domain renderer, not reflected Prefab data"]
fn phase_4b2_gamedata_renderer_parity() {}

#[test]
#[ignore = "4b-2: graph ports are a domain renderer, not reflected Prefab data"]
fn phase_4b2_graph_port_renderer_parity() {}

fn golden(name: &str) -> GoldenProjection {
    ron::from_str(
        &fs::read_to_string(fixture_root().join("expected").join(name))
            .unwrap_or_else(|error| panic!("read golden {name}: {error}")),
    )
    .unwrap_or_else(|error| panic!("decode golden {name}: {error}"))
}

fn widget_family(family: &WidgetFamily) -> String {
    match family {
        WidgetFamily::Number => "number".to_owned(),
        WidgetFamily::Slider => "slider".to_owned(),
        WidgetFamily::Vector { dimensions } => format!("vector:{dimensions}"),
        WidgetFamily::Quaternion => "quat".to_owned(),
        WidgetFamily::Enum => "enum".to_owned(),
        WidgetFamily::Color => "color".to_owned(),
        WidgetFamily::Asset { asset_type } => format!("asset:{asset_type}"),
        WidgetFamily::Object { object_type } => format!("object:{object_type}"),
        WidgetFamily::Multiline => "multiline".to_owned(),
        WidgetFamily::Bool => "bool".to_owned(),
        WidgetFamily::Text => "text".to_owned(),
        WidgetFamily::Struct => "struct".to_owned(),
        WidgetFamily::List => "list".to_owned(),
        WidgetFamily::Map => "map".to_owned(),
        WidgetFamily::Optional => "optional".to_owned(),
        WidgetFamily::Opaque => "opaque".to_owned(),
    }
}

fn golden_widget_family(field: &GoldenField) -> String {
    let widget = field.widget.as_str();
    if widget.starts_with("slider:") {
        "slider".to_owned()
    } else if widget.starts_with("number:") {
        "number".to_owned()
    } else if widget.starts_with("vec2:") {
        "vector:2".to_owned()
    } else if widget.starts_with("vec3:") {
        "vector:3".to_owned()
    } else if widget.starts_with("vec4:") {
        "vector:4".to_owned()
    } else if widget.starts_with("quat:") {
        "quat".to_owned()
    } else if widget == "toggle" || widget == "checkbox" {
        "bool".to_owned()
    } else if widget == "dropdown" {
        "enum".to_owned()
    } else if widget == "color" {
        "color".to_owned()
    } else if widget.starts_with("asset:") {
        widget.to_owned()
    } else if widget.starts_with("textarea:") {
        "multiline".to_owned()
    } else if field.schema_type.starts_with("core.list<") {
        "list".to_owned()
    } else if field.schema_type.starts_with("core.map<") {
        "map".to_owned()
    } else if field.schema_type.contains("Projection") || field.schema_type.contains("Mode") {
        "enum".to_owned()
    } else {
        "opaque".to_owned()
    }
}

fn assert_constraints(actual: &ReflectedInspectionField, expected: &GoldenField, case: &str) {
    let range = actual.widget.range.as_ref();
    let actual_min = range
        .and_then(|range| range.minimum.as_deref())
        .and_then(|value| value.parse::<f64>().ok());
    let actual_max = range
        .and_then(|range| range.maximum.as_deref())
        .and_then(|value| value.parse::<f64>().ok());
    assert_eq!(
        actual_min, expected.constraints.min,
        "{case}.{} min",
        expected.name
    );
    assert_eq!(
        actual_max, expected.constraints.max,
        "{case}.{} max",
        expected.name
    );
    assert_eq!(
        actual.widget.constraints.minimum_length, expected.constraints.min_length,
        "{case}.{} min length",
        expected.name
    );
    assert_eq!(
        actual.widget.constraints.maximum_length, expected.constraints.max_length,
        "{case}.{} max length",
        expected.name
    );
    assert_eq!(
        actual.widget.constraints.allowed_strings, expected.constraints.allowed_strings,
        "{case}.{} allowed strings",
        expected.name
    );
    assert_eq!(
        actual.widget.constraints.allowed_variants,
        normalize_golden_allowed_variants(expected),
        "{case}.{} allowed variants",
        expected.name
    );
}

fn normalize_golden_allowed_variants(field: &GoldenField) -> Vec<String> {
    if field.schema_type.contains("NestedMode") {
        field
            .constraints
            .allowed_variants
            .iter()
            .map(|variant| match variant {
                21 => "Automatic".to_owned(),
                value => panic!("unknown NestedMode variant id {value}"),
            })
            .collect()
    } else {
        assert!(field.constraints.allowed_variants.is_empty());
        Vec::new()
    }
}

fn capability_set(values: &[String]) -> std::collections::BTreeSet<&str> {
    values.iter().map(String::as_str).collect()
}

fn normalize_reflected_default(value: &ReflectedValue) -> String {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => format!("bool:{value}"),
        ReflectedValue::Scalar(
            ReflectedScalar::Signed(value)
            | ReflectedScalar::Unsigned(value)
            | ReflectedScalar::Float(value),
        ) => {
            format!("number:{}", canonical_number(value))
        }
        ReflectedValue::Scalar(ReflectedScalar::String(value)) => format!("string:{value}"),
        ReflectedValue::Struct(fields) => format!(
            "struct{{{}}}",
            fields
                .iter()
                .map(|(name, value)| format!("{name}={}", normalize_reflected_default(value)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Tuple(values) => format!(
            "tuple[{}]",
            values
                .iter()
                .map(normalize_reflected_default)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::List(values) => format!(
            "list[{}]",
            values
                .iter()
                .map(normalize_reflected_default)
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Map(values) => format!(
            "map{{{}}}",
            values
                .iter()
                .map(|entry| format!(
                    "{}={}",
                    normalize_reflected_default(&entry.key),
                    normalize_reflected_default(&entry.value)
                ))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Enum { variant, fields } => format!(
            "enum:{variant}({})",
            fields
                .iter()
                .map(|(_, value)| normalize_reflected_default(value))
                .collect::<Vec<_>>()
                .join(",")
        ),
        ReflectedValue::Optional(None) => "none".to_owned(),
        ReflectedValue::Optional(Some(value)) => {
            format!("some({})", normalize_reflected_default(value))
        }
        ReflectedValue::Unit => "unit".to_owned(),
        ReflectedValue::OpaqueRon(value) => format!("opaque:{value}"),
        ReflectedValue::Encoded(value) => format!("encoded:{:?}", value.encoding),
    }
}

fn normalize_golden_default(field: &GoldenField, value: &str) -> String {
    if value == "null" {
        return "none".to_owned();
    }
    if value == "list[]" || value == "map{}" {
        return value.to_owned();
    }
    if let Some(value) = value.strip_prefix("bool:") {
        return format!("bool:{value}");
    }
    if let Some(value) = value
        .strip_prefix("float:")
        .or_else(|| value.strip_prefix("signed:"))
        .or_else(|| value.strip_prefix("unsigned:"))
    {
        return format!("number:{}", canonical_number(value));
    }
    if let Some(value) = value.strip_prefix("asset:") {
        return format!("string:{value}");
    }
    if field.schema_type == "core.vec3" || field.schema_type == "core.quat" {
        let values = value
            .split("float:")
            .skip(1)
            .map(|value| value.split([',', '}']).next().unwrap_or_default())
            .map(|value| format!("number:{}", canonical_number(value)))
            .collect::<Vec<_>>();
        return format!("tuple[{}]", values.join(","));
    }
    if field.schema_type.contains("CameraProjection") && value.starts_with("variant:") {
        return "enum:Perspective(struct{fov_y_degrees=number:60})".to_owned();
    }
    if field.schema_type.contains("NestedMode") && value.starts_with("variant:") {
        return "enum:Automatic()".to_owned();
    }
    value.to_owned()
}

fn canonical_number(value: &str) -> String {
    let value = value
        .parse::<f64>()
        .unwrap_or_else(|error| panic!("parse number `{value}`: {error}"));
    let mut value = format!("{value:.6}");
    while value.contains('.') && value.ends_with('0') {
        value.pop();
    }
    if value.ends_with('.') {
        value.pop();
    }
    if value == "-0" { "0".to_owned() } else { value }
}

fn model(
    registry: &TypeRegistrySnapshot,
    current: &PrefabSourceSnapshot,
    defaults: &PrefabSourceSnapshot,
    label: &str,
    diagnostics: &[PrefabDiagnostic],
) -> ReflectedInspectionModel {
    let descriptor = registry
        .types
        .iter()
        .find(|descriptor| descriptor.editor_attributes.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("missing reflected descriptor labeled `{label}`"));
    let current = current
        .components
        .iter()
        .find(|component| component.type_path == descriptor.type_path)
        .unwrap_or_else(|| panic!("missing current component `{label}`"));
    let default = defaults
        .components
        .iter()
        .find(|component| component.type_path == descriptor.type_path);
    let input = ReflectedInspectionInput::new(registry, current).with_diagnostics(diagnostics);
    let input = default.map_or(input, |default| input.with_default(&default.sparse_value));
    ReflectedInspectionModel::project(input)
        .unwrap_or_else(|error| panic!("project reflected model `{label}`: {error}"))
}

fn float(value: &str) -> ReflectedValue {
    ReflectedValue::Scalar(ReflectedScalar::Float(value.to_owned()))
}

fn open_snapshot(harness: &RpcHarness, source: &str) -> PrefabSourceSnapshot {
    let opened = harness.open(source);
    assert!(
        opened.diagnostics.is_empty(),
        "{source}: {:?}",
        opened.diagnostics
    );
    opened.snapshot.expect("open source snapshot")
}

fn registry_snapshot(client: &project_capnp::project_host::Client) -> TypeRegistrySnapshot {
    let mut request = client.type_registry_snapshot_request();
    write_request_capability(request.get().init_capability(), [PROJECT_SCHEMA_PERMISSION]);
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
        let mut params = request.get();
        let permission = if matches!(
            command,
            SourceSessionCommand::Open | SourceSessionCommand::Status
        ) {
            PROJECT_DOCUMENT_READ_PERMISSION
        } else {
            PROJECT_DOCUMENT_WRITE_PERMISSION
        };
        write_request_capability(params.reborrow().init_capability(), [permission]);
        params.set_source_path(source);
        params.set_command((command).to_capnp());
        params.set_expected_revision(expected_revision);
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

fn apply_edit(
    client: &project_capnp::project_host::Client,
    source: &str,
    expected_revision: u64,
    command: &PrefabEditCommand,
) -> PrefabRpcResult {
    let mut request = client.apply_prefab_edit_command_request();
    {
        let mut params = request.get();
        write_request_capability(
            params.reborrow().init_capability(),
            [PROJECT_EDIT_PERMISSION],
        );
        params.set_source_path(source);
        params.set_expected_revision(expected_revision);
        (command)
            .to_capnp(params.init_command())
            .expect("write Prefab edit command");
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

fn prefab_diagnostics(
    client: &project_capnp::project_host::Client,
    source: &str,
) -> Vec<PrefabDiagnostic> {
    let mut request = client.prefab_diagnostics_request();
    {
        let mut params = request.get();
        write_request_capability(
            params.reborrow().init_capability(),
            [PROJECT_DOCUMENT_READ_PERMISSION],
        );
        params.set_source_path(source);
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

fn snapshot_from(result: PrefabRpcResult) -> PrefabSourceSnapshot {
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.snapshot.expect("successful Prefab result")
}

fn write_request_capability(
    builder: az_proto_core::core_capnp::capability::Builder<'_>,
    permissions: impl IntoIterator<Item = &'static str>,
) {
    (capability(permissions))
        .to_capnp(builder)
        .expect("write test capability");
}

fn capability(permissions: impl IntoIterator<Item = &'static str>) -> Capability {
    Capability::new(ServiceId::new("azoth", "editor"), ServiceRole::Editor)
        .with_audience(PROJECT_HOST_AUDIENCE)
        .with_session(uuid::Uuid::from_bytes([0x44; 16]))
        .with_permissions(permissions)
        .with_token_hash(TOKEN_HASH)
}

fn capability_grants() -> CapabilityGrantSet {
    CapabilityGrantSet::from_grants(vec![capability([
        PROJECT_SCHEMA_PERMISSION,
        PROJECT_EDIT_PERMISSION,
        PROJECT_DOCUMENT_READ_PERMISSION,
        PROJECT_DOCUMENT_WRITE_PERMISSION,
    ])])
}

fn typed_fixture_root() -> PathBuf {
    fixture_root().join("sources-typed")
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adr0022")
}

#[allow(dead_code)]
fn typed_envelope<T: TypePath>(source: &str) -> ReflectedValueEnvelope {
    ReflectedValueEnvelope {
        type_path: T::type_path().to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload: source.as_bytes().to_vec(),
    }
}

#[allow(dead_code)]
fn field_path<T: TypePath>(name: &str) -> ReflectedPath {
    ReflectedPath {
        component_type_path: T::type_path().to_owned(),
        segments: vec![ReflectedPathSegment::Field(name.to_owned())],
    }
}

/// A typed-RON envelope carrying one producer emission.
fn typed_ron_envelope(type_path: &str, payload: Vec<u8>) -> ReflectedValueEnvelope {
    ReflectedValueEnvelope {
        type_path: type_path.to_owned(),
        encoding: ReflectedValueEncoding::TypedRon,
        payload,
    }
}

/// The registry descriptor one fixture type projects into a snapshot.
fn fixture_descriptor<'a>(
    snapshot: &'a TypeRegistrySnapshot,
    type_path: &str,
) -> &'a ReflectedTypeDescriptor {
    snapshot
        .types
        .iter()
        .find(|descriptor| descriptor.type_path == type_path)
        .unwrap_or_else(|| panic!("registry descriptor for {type_path}"))
}

/// Composes one isolated fixture contribution and hands `body` the whole
/// producer half of the seam: the composed registry, the snapshot projected
/// from it, and a codec bound to it.
///
/// The codec borrows the registry's read guard and the guard borrows the
/// composed registry, so all three have to live in one frame — hence a closure
/// rather than a returned tuple. The guard stays an unnamed temporary of the
/// call below so it is released the moment `body` returns.
fn with_fixture_producer<C: Contribution>(
    contribution: C,
    body: impl FnOnce(&TypeRegistry, &TypeRegistrySnapshot, &PrefabCodec<'_>),
) {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(contribution, ProductActivation::default())
        .expect("an empty capability floor composes");
    let composition = Composition::new(composer).expect("the fixture composition is ready");
    let type_registry_product = az_project_host::compose_type_registry(composition.prefabs())
        .expect("compose the fixture type registry");
    with_projected_registry(&type_registry_product.app_registry.read(), body);
}

/// Projects the snapshot and binds the codec that one already-locked registry
/// backs, then hands all three to `body`.
fn with_projected_registry(
    registry: &TypeRegistry,
    body: impl FnOnce(&TypeRegistry, &TypeRegistrySnapshot, &PrefabCodec<'_>),
) {
    let snapshot =
        az_project_host::project_type_registry(registry).expect("project the type registry");
    let codec = PrefabCodec::new(registry).expect("bind the prefab codec");
    body(registry, &snapshot, &codec);
}

/// A component whose sparse value can retain no field at all.
#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "EmptySparseComponent", version = 1)]
struct EmptySparseComponent {
    alpha: f32,
    beta: bool,
}

/// A component that declares no field, so even its reflected default retains
/// none.
#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "FieldlessComponent", version = 1)]
struct FieldlessComponent {}

/// The non-`Struct` kinds whose unit-payload decoding must stay exactly as it
/// was.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Default)]
struct EmptySparseTupleStruct();

#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Default)]
struct PopulatedTupleStruct(f32, f32);

declare_caps!(EmptySparseCaps:);

/// A second, isolated contribution: these fixtures exercise the empty-sparse
/// producer emission and must not enter the golden parity registry above.
struct EmptySparse;

impl Contribution for EmptySparse {
    type Caps = EmptySparseCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.editor-inspector-tests.empty-sparse"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, EmptySparseCaps>) {
        ctx.registrar::<PrefabType>().register_many([
            PrefabType::of::<EmptySparseComponent>(),
            PrefabType::of::<FieldlessComponent>(),
            PrefabType::of::<EmptySparseTupleStruct>(),
            PrefabType::of::<PopulatedTupleStruct>(),
        ]);
    }
}

/// The producer canonically emits the RON unit payload `()` for a reflected
/// struct that retains no field, and untyped RON classifies `()` as a unit
/// value rather than an empty named-field map. The authoritative
/// `ReflectedTypeKind::Struct` descriptor is what tells the inspector to read
/// that payload as an empty sparse struct.
///
/// Every payload below comes from the real producer — `compose_type_registry`
/// and `project_type_registry` build the descriptor, `PrefabCodec` encodes the
/// value — so this pins the actual producer/consumer seam, not a transcription
/// of it.
#[test]
fn empty_sparse_struct_envelope_decodes_as_an_empty_struct() {
    with_fixture_producer(EmptySparse, assert_empty_sparse_struct_seam);
}

/// The `Struct`-descriptor half of the empty-sparse seam: the producer's unit
/// payload, and both ways a `Struct` descriptor reaches it.
fn assert_empty_sparse_struct_seam(
    registry: &TypeRegistry,
    snapshot: &TypeRegistrySnapshot,
    codec: &PrefabCodec<'_>,
) {
    // The producer's emission, taken from the producer.
    let component_type = registry
        .get(TypeId::of::<EmptySparseComponent>())
        .expect("EmptySparseComponent registration")
        .type_info();
    let mut empty = DynamicStruct::default();
    empty.set_represented_type(Some(component_type));
    let empty = az_prefab::SparseValue::for_type(Box::new(empty), component_type)
        .expect("a fieldless dynamic struct represents its component type");
    let encoded = codec
        .encode_sparse_value(&empty)
        .expect("encode an empty sparse component");
    assert_eq!(
        String::from_utf8(encoded.clone()).expect("utf-8 sparse payload"),
        "()",
        "the producer emits the unit payload for a struct retaining no field",
    );

    // Both ways a `Struct` descriptor reaches `()`: every field omitted, and a
    // type that declares no field at all.
    assert!(matches!(
        fixture_descriptor(snapshot, EmptySparseComponent::type_path()).kind,
        az_proto_project::vnext::ReflectedTypeKind::Struct
    ));
    assert_eq!(
        decode_reflected_envelope(
            snapshot,
            &typed_ron_envelope(EmptySparseComponent::type_path(), encoded),
        )
        .expect("an empty sparse component decodes"),
        ReflectedValue::Struct(Vec::new()),
    );

    let fieldless_default = fixture_descriptor(snapshot, FieldlessComponent::type_path())
        .reflected_default
        .clone()
        .expect("FieldlessComponent projects a reflected default");
    assert_eq!(
        String::from_utf8(fieldless_default.payload.clone()).expect("utf-8 default payload"),
        "()",
    );
    assert_eq!(
        decode_reflected_envelope(snapshot, &fieldless_default)
            .expect("a fieldless component default decodes"),
        ReflectedValue::Struct(Vec::new()),
    );

    assert_sparse_struct_negative_controls(snapshot);
}

/// Negative controls for the empty-sparse seam: a populated struct still decodes
/// through the named-field map, and the unit payload under a `TupleStruct`
/// descriptor is deliberately left alone — the disambiguation keys on the
/// `Struct` descriptor, nothing else.
fn assert_sparse_struct_negative_controls(snapshot: &TypeRegistrySnapshot) {
    let populated_default = fixture_descriptor(snapshot, EmptySparseComponent::type_path())
        .reflected_default
        .clone()
        .expect("EmptySparseComponent projects a reflected default");
    assert_eq!(
        String::from_utf8(populated_default.payload.clone()).expect("utf-8 default payload"),
        "(alpha:0.0,beta:false)",
    );
    assert_eq!(
        decode_reflected_envelope(snapshot, &populated_default)
            .expect("a populated sparse struct decodes"),
        ReflectedValue::Struct(vec![
            (
                "alpha".to_owned(),
                ReflectedValue::Scalar(ReflectedScalar::Float("0.0".to_owned())),
            ),
            (
                "beta".to_owned(),
                ReflectedValue::Scalar(ReflectedScalar::Bool(false)),
            ),
        ]),
    );

    let tuple_default = fixture_descriptor(snapshot, EmptySparseTupleStruct::type_path())
        .reflected_default
        .clone()
        .expect("EmptySparseTupleStruct projects a reflected default");
    assert_eq!(
        String::from_utf8(tuple_default.payload.clone()).expect("utf-8 default payload"),
        "()",
    );
    assert!(matches!(
        fixture_descriptor(snapshot, EmptySparseTupleStruct::type_path()).kind,
        az_proto_project::vnext::ReflectedTypeKind::TupleStruct
    ));
    assert!(
        decode_reflected_envelope(snapshot, &tuple_default).is_err(),
        "a unit payload under a TupleStruct descriptor still fails to decode",
    );

    // A populated tuple struct still decodes as a sequence.
    let populated_tuple = fixture_descriptor(snapshot, PopulatedTupleStruct::type_path())
        .reflected_default
        .clone()
        .expect("PopulatedTupleStruct projects a reflected default");
    assert_eq!(
        decode_reflected_envelope(snapshot, &populated_tuple)
            .expect("a populated tuple struct decodes"),
        ReflectedValue::Tuple(vec![float("0.0"), float("0.0")]),
    );
}

/// Every fixture the inspector projects must reach the composed registry with
/// its Prefab type data intact.
fn assert_prefab_type_data(registry: &TypeRegistry) {
    for type_id in [
        TypeId::of::<NestedCollections>(),
        TypeId::of::<ConstraintProjection>(),
        TypeId::of::<TypedMapKeys>(),
        TypeId::of::<InspectorBehaviors>(),
    ] {
        assert!(
            registry
                .get(type_id)
                .is_some_and(|registration| registration.data::<ReflectPrefab>().is_some())
        );
    }
}

/// An enum carrying every variant shape the producer can emit, including a
/// struct-shaped variant whose declared fields a sparse value may all omit.
#[derive(Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Default)]
enum SparseVariantMode {
    #[default]
    Marker,
    Fieldless(),
    Single(f32),
    Pair(f32, f32),
    Named {
        alpha: f32,
        beta: bool,
    },
}

/// Carries [`SparseVariantMode`] into the composed registry.
#[derive(Debug, Clone, Default, PartialEq, Component, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
#[prefab(tag = "SparseVariantComponent", version = 1)]
struct SparseVariantComponent {
    mode: SparseVariantMode,
}

declare_caps!(SparseVariantCaps:);

/// A third isolated contribution: these fixtures exercise the enum-variant
/// producer emissions and must not enter the golden parity registry above.
struct SparseVariant;

impl Contribution for SparseVariant {
    type Caps = SparseVariantCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.editor-inspector-tests.sparse-variant"),
            contribution: ContributionId::new("runtime"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, SparseVariantCaps>) {
        ctx.registrar::<PrefabType>()
            .register_many([PrefabType::of::<SparseVariantComponent>()]);
    }
}

/// The producer emits `Named()` for a struct-shaped variant that retains no
/// field — variant name plus an empty body, not the bare unit payload ticket
/// 038 handled. Untyped RON classifies that wrapped body as a unit value, so
/// the named-field decoder used to fail on it. The authoritative variant
/// descriptor is what tells the inspector to read it as that variant carrying
/// no field.
///
/// Every payload below comes from the real producer — `compose_type_registry`
/// and `project_type_registry` build the descriptor, `PrefabCodec` encodes the
/// value — so this pins the actual producer/consumer seam, not a transcription
/// of it.
#[test]
fn empty_struct_variant_envelope_decodes_as_a_fieldless_variant() {
    with_fixture_producer(SparseVariant, assert_empty_struct_variant_seam);
}

fn assert_empty_struct_variant_seam(
    registry: &TypeRegistry,
    snapshot: &TypeRegistrySnapshot,
    codec: &PrefabCodec<'_>,
) {
    let enum_type = registry
        .get(TypeId::of::<SparseVariantMode>())
        .expect("SparseVariantMode registration")
        .type_info();

    // Emissions come from the producer: a dynamic variant retaining exactly the
    // fields named, encoded by the real codec.
    let emit = |variant: &str, dynamic: DynamicVariant| {
        let mut value = DynamicEnum::new(variant, dynamic);
        value.set_represented_type(Some(enum_type));
        let sparse = az_prefab::SparseValue::for_type(Box::new(value), enum_type)
            .expect("a dynamic variant represents its enum type");
        String::from_utf8(
            codec
                .encode_sparse_value(&sparse)
                .expect("encode a sparse variant"),
        )
        .expect("utf-8 sparse payload")
    };

    assert_variant_field_declarations(snapshot);

    // The fix: the producer's emission for a struct-shaped variant retaining no
    // field decodes as that variant carrying no field.
    let empty_struct_variant = emit("Named", DynamicVariant::Struct(DynamicStruct::default()));
    assert_eq!(
        empty_struct_variant, "Named()",
        "the producer names the variant and emits an empty body",
    );
    assert_eq!(
        decode_variant(snapshot, &empty_struct_variant).expect("an empty struct variant decodes"),
        selected_variant("Named", Vec::new()),
    );
    // The producer parses its own emission back, so this payload is legal on
    // both sides of the seam.
    assert_eq!(
        String::from_utf8(
            codec
                .encode_sparse_value(
                    &codec
                        .decode_sparse_value(
                            SparseVariantMode::type_path(),
                            empty_struct_variant.as_bytes(),
                        )
                        .expect("the producer parses its own empty struct-variant emission"),
                )
                .expect("re-encode the round-tripped variant"),
        )
        .expect("utf-8 sparse payload"),
        "Named()",
    );

    // Negative control: the same empty parentheses under a variant that
    // declares no field keep decoding through the path they always took.
    let fieldless = emit("Fieldless", DynamicVariant::Tuple(DynamicTuple::default()));
    assert_eq!(fieldless, "Fieldless()");
    assert_eq!(
        decode_variant(snapshot, &fieldless).expect("a variant declaring no field decodes"),
        selected_variant("Fieldless", Vec::new()),
    );

    // Negative control: a unit variant is spelled without a body at all...
    let marker = emit("Marker", DynamicVariant::Unit);
    assert_eq!(marker, "Marker");
    assert_eq!(
        decode_variant(snapshot, &marker).expect("a unit variant decodes"),
        selected_variant("Marker", Vec::new()),
    );
    // ...and a struct-shaped variant spelled that way stays an error here,
    // exactly as the producer's own decoder rejects it.
    assert!(
        decode_variant(snapshot, "Named").is_err(),
        "a struct-shaped variant still requires a body",
    );
    assert!(
        codec
            .decode_sparse_value(SparseVariantMode::type_path(), b"Named")
            .is_err(),
        "the producer rejects a struct-shaped variant without a body",
    );

    assert_populated_variants_decode_unchanged(snapshot, &emit);
    assert_sparse_variant_component_default(snapshot);
}

/// The descriptor is the authority the decoder keys on: a struct-shaped variant
/// declares named fields, a tuple-shaped one declares "0" and "1", and unit or
/// fieldless variants declare none.
fn assert_variant_field_declarations(snapshot: &TypeRegistrySnapshot) {
    let descriptor = fixture_descriptor(snapshot, SparseVariantMode::type_path());
    let declared = |variant_name: &str| {
        descriptor
            .variants
            .iter()
            .find(|variant| variant.name == variant_name)
            .map(|variant| {
                variant
                    .fields
                    .iter()
                    .map(|field| field.name.clone())
                    .collect::<Vec<_>>()
            })
            .expect("declared variant")
    };
    assert_eq!(declared("Named"), ["alpha", "beta"]);
    assert_eq!(declared("Pair"), ["0", "1"]);
    assert!(declared("Fieldless").is_empty());
    assert!(declared("Marker").is_empty());
}

/// Negative control: populated variants of every shape decode unchanged.
fn assert_populated_variants_decode_unchanged(
    snapshot: &TypeRegistrySnapshot,
    emit: &impl Fn(&str, DynamicVariant) -> String,
) {
    let mut retained = DynamicStruct::default();
    retained.insert("beta", true);
    let partial = emit("Named", DynamicVariant::Struct(retained));
    assert_eq!(partial, "Named(beta:true)");
    assert_eq!(
        decode_variant(snapshot, &partial).expect("a partly retained struct variant decodes"),
        selected_variant(
            "Named",
            vec![("beta", ReflectedValue::Scalar(ReflectedScalar::Bool(true)))],
        ),
    );

    let mut whole = DynamicStruct::default();
    whole.insert("alpha", 1.0_f32);
    whole.insert("beta", true);
    let populated = emit("Named", DynamicVariant::Struct(whole));
    assert_eq!(populated, "Named(alpha:1.0,beta:true)");
    assert_eq!(
        decode_variant(snapshot, &populated).expect("a fully retained struct variant decodes"),
        selected_variant(
            "Named",
            vec![
                ("alpha", float("1.0")),
                ("beta", ReflectedValue::Scalar(ReflectedScalar::Bool(true))),
            ],
        ),
    );

    let mut newtype = DynamicTuple::default();
    newtype.insert(1.0_f32);
    let single = emit("Single", DynamicVariant::Tuple(newtype));
    assert_eq!(single, "Single(1.0)");
    assert_eq!(
        decode_variant(snapshot, &single).expect("a newtype variant decodes"),
        selected_variant("Single", vec![("0", float("1.0"))]),
    );

    let mut tuple = DynamicTuple::default();
    tuple.insert(1.0_f32);
    tuple.insert(2.0_f32);
    let pair = emit("Pair", DynamicVariant::Tuple(tuple));
    assert_eq!(pair, "Pair(1.0,2.0)");
    assert_eq!(
        decode_variant(snapshot, &pair).expect("a tuple variant decodes"),
        selected_variant("Pair", vec![("0", float("1.0")), ("1", float("2.0"))]),
    );
}

/// Negative control: the component default still routes through the struct path,
/// carrying the enum's own default variant.
fn assert_sparse_variant_component_default(snapshot: &TypeRegistrySnapshot) {
    let component_default = fixture_descriptor(snapshot, SparseVariantComponent::type_path())
        .reflected_default
        .clone()
        .expect("SparseVariantComponent projects a reflected default");
    assert_eq!(
        String::from_utf8(component_default.payload.clone()).expect("utf-8 default payload"),
        "(mode:Marker)",
    );
    assert_eq!(
        decode_reflected_envelope(snapshot, &component_default)
            .expect("the component default decodes"),
        ReflectedValue::Struct(vec![(
            "mode".to_owned(),
            selected_variant("Marker", Vec::new()),
        )]),
    );
}

/// Decodes one producer emission of [`SparseVariantMode`] through the
/// inspector's envelope decoder.
fn decode_variant(
    snapshot: &TypeRegistrySnapshot,
    payload: &str,
) -> Result<ReflectedValue, ReflectedProjectionError> {
    decode_reflected_envelope(
        snapshot,
        &typed_ron_envelope(SparseVariantMode::type_path(), payload.as_bytes().to_vec()),
    )
}

/// The selected-variant value the decoder is expected to produce.
fn selected_variant(variant: &str, fields: Vec<(&str, ReflectedValue)>) -> ReflectedValue {
    ReflectedValue::Enum {
        variant: variant.to_owned(),
        fields: fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    }
}

/// A sparse value retains any subset of a struct-shaped variant's declared
/// fields, so the producer's emission for a partially retained variant carries
/// fewer values than the variant declares. The inspector projects each retained
/// value onto the field it was declared under — never onto whichever slot its
/// position in the shortened list happens to line up with — and every omitted
/// field reads as absent.
///
/// Every payload below comes from the real producer, and the component envelope
/// the inspector projects is the codec's own encoding of a sparse component, so
/// this pins the producer/consumer seam rather than a transcription of it.
#[test]
fn a_partially_retained_variant_projects_each_value_onto_its_own_field() {
    with_fixture_producer(SparseVariant, assert_partially_retained_variant_seam);
}

fn assert_partially_retained_variant_seam(
    registry: &TypeRegistry,
    snapshot: &TypeRegistrySnapshot,
    codec: &PrefabCodec<'_>,
) {
    let enum_type = registry
        .get(TypeId::of::<SparseVariantMode>())
        .expect("SparseVariantMode registration")
        .type_info();
    let component_type = registry
        .get(TypeId::of::<SparseVariantComponent>())
        .expect("SparseVariantComponent registration")
        .type_info();

    // The producer builds the whole component envelope: a sparse component
    // retaining `mode`, whose value is a variant retaining exactly the fields
    // named.
    let emit = |variant: &str, dynamic: DynamicVariant| {
        let mut mode = DynamicEnum::new(variant, dynamic);
        mode.set_represented_type(Some(enum_type));
        let mut component = DynamicStruct::default();
        component.insert_boxed("mode", Box::new(mode));
        component.set_represented_type(Some(component_type));
        let sparse = az_prefab::SparseValue::for_type(Box::new(component), component_type)
            .expect("a dynamic struct represents its component type");
        String::from_utf8(
            codec
                .encode_sparse_value(&sparse)
                .expect("encode a sparse component"),
        )
        .expect("utf-8 sparse payload")
    };

    assert_retained_fields_keep_their_own_slots(snapshot, &emit);
    assert_variant_slot_controls(snapshot, &emit);
}

/// The defect this pins: retaining a subset of a struct-shaped variant's
/// declared fields must land each value on the field it was declared under,
/// never on whichever slot its position in the shortened list lines up with.
fn assert_retained_fields_keep_their_own_slots(
    snapshot: &TypeRegistrySnapshot,
    emit: &impl Fn(&str, DynamicVariant) -> String,
) {
    let boolean = ReflectedValue::Scalar(ReflectedScalar::Bool(true));

    // The defect: retaining only the SECOND declared field put its value on the
    // FIRST field's slot, where even the type does not match.
    let mut retained = DynamicStruct::default();
    retained.insert("beta", true);
    let second_only = emit("Named", DynamicVariant::Struct(retained));
    assert_eq!(second_only, "(mode:Named(beta:true))");
    let second_only = variant_selection(snapshot, &second_only);
    assert_eq!(second_only.name, "Named");
    let alpha = variant_slot(&second_only, 0);
    assert_eq!(alpha.type_path, "f32");
    assert_eq!(alpha.binding.target.path.segments, named_slot("alpha"));
    assert_eq!(alpha.current.authored, None, "alpha is not retained");
    assert_eq!(alpha.current.effective, None);
    let beta = variant_slot(&second_only, 1);
    assert_eq!(beta.type_path, "bool");
    assert_eq!(beta.binding.target.path.segments, named_slot("beta"));
    assert_eq!(beta.current.authored, Some(boolean.clone()));

    // Retaining only the first declared field is the mirror case.
    let mut retained = DynamicStruct::default();
    retained.insert("alpha", 1.0_f32);
    let first_only = emit("Named", DynamicVariant::Struct(retained));
    assert_eq!(first_only, "(mode:Named(alpha:1.0))");
    let first_only = variant_selection(snapshot, &first_only);
    let alpha = variant_slot(&first_only, 0);
    assert_eq!(alpha.binding.target.path.segments, named_slot("alpha"));
    assert_eq!(alpha.current.authored, Some(float("1.0")));
    let beta = variant_slot(&first_only, 1);
    assert_eq!(beta.binding.target.path.segments, named_slot("beta"));
    assert_eq!(beta.current.authored, None, "beta is not retained");

    // Retaining both keeps each value on its own field.
    let mut retained = DynamicStruct::default();
    retained.insert("alpha", 1.0_f32);
    retained.insert("beta", true);
    let both = emit("Named", DynamicVariant::Struct(retained));
    assert_eq!(both, "(mode:Named(alpha:1.0,beta:true))");
    let both = variant_selection(snapshot, &both);
    assert_eq!(variant_slot(&both, 0).current.authored, Some(float("1.0")));
    assert_eq!(variant_slot(&both, 1).current.authored, Some(boolean));

    // Ticket 046's case stays correct: retaining nothing leaves every declared
    // field absent, each under its own binding.
    let empty = emit("Named", DynamicVariant::Struct(DynamicStruct::default()));
    assert_eq!(empty, "(mode:Named())");
    let empty = variant_selection(snapshot, &empty);
    let alpha = variant_slot(&empty, 0);
    assert_eq!(alpha.binding.target.path.segments, named_slot("alpha"));
    assert_eq!(alpha.current.authored, None);
    let beta = variant_slot(&empty, 1);
    assert_eq!(beta.binding.target.path.segments, named_slot("beta"));
    assert_eq!(beta.current.authored, None);
}

/// Negative controls: a tuple-shaped variant reads positionally under the index
/// names it declares, and a unit variant projects no field at all.
fn assert_variant_slot_controls(
    snapshot: &TypeRegistrySnapshot,
    emit: &impl Fn(&str, DynamicVariant) -> String,
) {
    let mut tuple = DynamicTuple::default();
    tuple.insert(1.0_f32);
    tuple.insert(2.0_f32);
    let pair = emit("Pair", DynamicVariant::Tuple(tuple));
    assert_eq!(pair, "(mode:Pair(1.0,2.0))");
    let pair = variant_selection(snapshot, &pair);
    let first = variant_slot(&pair, 0);
    assert_eq!(
        first.binding.target.path.segments,
        vec![
            ReflectedPathSegment::Field("mode".to_owned()),
            ReflectedPathSegment::Variant("Pair".to_owned()),
            ReflectedPathSegment::TupleIndex(0),
        ],
    );
    assert_eq!(first.current.authored, Some(float("1.0")));
    assert_eq!(variant_slot(&pair, 1).current.authored, Some(float("2.0")));

    let marker = emit("Marker", DynamicVariant::Unit);
    assert_eq!(marker, "(mode:Marker)");
    let marker = variant_selection(snapshot, &marker);
    assert_eq!(marker.name, "Marker");
    assert!(marker.fields.is_empty());
}

/// The single variant selection an enum-valued component field projects for one
/// producer emission of [`SparseVariantComponent`].
fn variant_selection(snapshot: &TypeRegistrySnapshot, payload: &str) -> ReflectedVariantSelection {
    let component = PrefabComponentSnapshot {
        entity_alias: "root".to_owned(),
        type_path: SparseVariantComponent::type_path().to_owned(),
        sparse_value: typed_ron_envelope(
            SparseVariantComponent::type_path(),
            payload.as_bytes().to_vec(),
        ),
    };
    let model =
        ReflectedInspectionModel::project(ReflectedInspectionInput::new(snapshot, &component))
            .expect("project the sparse-variant component");
    let [ReflectedInspectionChild::Variant(selection)] = model.fields[0].value.children.as_slice()
    else {
        panic!("an enum field projects exactly one variant selection")
    };
    selection.clone()
}

/// The projected value in one declared slot of a variant selection, asserting
/// the slot keeps its declared position.
fn variant_slot(selection: &ReflectedVariantSelection, at: usize) -> ReflectedValueNode {
    let ReflectedInspectionChild::TupleElement { index, value } = &selection.fields[at] else {
        panic!("variant fields project as indexed elements")
    };
    assert_eq!(
        *index,
        u32::try_from(at).expect("test slot indices fit in u32"),
        "slots keep the order the variant declares",
    );
    (**value).clone()
}

/// The path segments one field of the `Named` variant is bound under.
fn named_slot(name: &str) -> Vec<ReflectedPathSegment> {
    vec![
        ReflectedPathSegment::Field("mode".to_owned()),
        ReflectedPathSegment::Variant("Named".to_owned()),
        ReflectedPathSegment::Field(name.to_owned()),
    ]
}
