//! ADR 0022 vNext projection and command contract.
//!
//! These types freeze the Phase 0 design vocabulary. They are deliberately
//! independent of generated Cap'n Proto bindings. Phase 4a exposes additive
//! RPCs while the supported/default editor route remains gated until cutover.

use std::collections::BTreeMap;

pub use az_core::reflect::{ReflectedValueEncoding, ReflectedValueEnvelope};
pub use az_proto_asset::{SourceFileEditOperation, SourceFileEditSnapshot, WorkspaceSourceFileRef};
use az_proto_core::Capability;

pub const CONTRACT_DESIGN_VERSION: u32 = 1;
pub const SUPPORTED_RPC_ROUTING_ENABLED: bool = true;
pub const CAPNP_SCHEMA: &str = include_str!("../schema/azoth/project.capnp");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRegistrySnapshot {
    pub schema_catalog_hash: Vec<u8>,
    pub types: Vec<ReflectedTypeDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedTypeDescriptor {
    pub type_path: String,
    pub short_path: String,
    pub kind: ReflectedTypeKind,
    pub fields: Vec<ReflectedFieldDescriptor>,
    pub variants: Vec<ReflectedVariantDescriptor>,
    pub editor_attributes: EditorAttributes,
    pub type_data_flags: Vec<String>,
    pub applicability: ApplicabilityDescriptor,
    pub reflected_default: Option<ReflectedValueEnvelope>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplicabilityDescriptor {
    pub provides: Vec<String>,
    pub requires: Vec<String>,
    pub incompatible: Vec<String>,
    pub default_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedTypeKind {
    Struct,
    TupleStruct,
    Tuple,
    List,
    Array { capacity: u32 },
    Map,
    Set,
    Enum,
    Optional,
    Bool,
    SignedInteger { bits: u8 },
    UnsignedInteger { bits: u8 },
    Float { bits: u8 },
    String,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedFieldDescriptor {
    pub name: String,
    pub type_path: String,
    pub editor_attributes: EditorAttributes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedVariantDescriptor {
    pub name: String,
    pub fields: Vec<ReflectedFieldDescriptor>,
    pub editor_attributes: EditorAttributes,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorAttributes {
    pub label: Option<String>,
    pub description: Option<String>,
    pub category: Option<String>,
    pub icon: Option<String>,
    pub widget: Option<String>,
    pub range: Option<NumericRange>,
    pub read_only: bool,
    pub hidden: bool,
    pub action_ids: Vec<String>,
    pub constraints: FieldConstraints,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldConstraints {
    pub minimum_length: Option<u32>,
    pub maximum_length: Option<u32>,
    pub allowed_strings: Vec<String>,
    pub allowed_variants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NumericRange {
    pub minimum: Option<String>,
    pub maximum: Option<String>,
    pub step: Option<String>,
    pub suffix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectedPath {
    pub component_type_path: String,
    pub segments: Vec<ReflectedPathSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReflectedPathSegment {
    Field(String),
    Variant(String),
    TupleIndex(u32),
    ListIndex(u32),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabSourceSnapshot {
    pub document_version: u32,
    pub type_versions: BTreeMap<String, u32>,
    pub entities: Vec<PrefabEntitySnapshot>,
    pub hierarchy: Vec<PrefabHierarchyEdge>,
    pub components: Vec<PrefabComponentSnapshot>,
    pub instances: Vec<PrefabInstanceSnapshot>,
    pub overrides: Vec<PrefabOverrideSnapshot>,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabEntitySnapshot {
    pub alias: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabHierarchyEdge {
    pub child_alias: String,
    pub parent_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabComponentSnapshot {
    pub entity_alias: String,
    pub type_path: String,
    pub sparse_value: ReflectedValueEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabInstanceSnapshot {
    pub alias: String,
    pub source_asset: String,
    pub parent_entity_alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabOverrideSnapshot {
    pub operation: PrefabOverrideOperation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefabOverrideOperation {
    Set {
        target: PrefabValueTarget,
        value: ReflectedValueEnvelope,
    },
    Clear {
        target: PrefabValueTarget,
    },
    Insert {
        target: PrefabValueTarget,
        index: u32,
        value: ReflectedValueEnvelope,
    },
    Remove {
        target: PrefabValueTarget,
        index: u32,
    },
    Move {
        target: PrefabValueTarget,
        from: u32,
        to: u32,
    },
}

impl PrefabOverrideOperation {
    #[must_use]
    pub const fn target(&self) -> &PrefabValueTarget {
        match self {
            Self::Set { target, .. }
            | Self::Clear { target }
            | Self::Insert { target, .. }
            | Self::Remove { target, .. }
            | Self::Move { target, .. } => target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabValueTarget {
    pub instance_alias_chain: Vec<String>,
    pub entity_alias: String,
    pub path: ReflectedPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefabEditCommand {
    SetValue {
        target: PrefabValueTarget,
        value: ReflectedValueEnvelope,
    },
    ListInsert {
        target: PrefabValueTarget,
        index: u32,
        value: ReflectedValueEnvelope,
    },
    ListRemove {
        target: PrefabValueTarget,
        index: u32,
    },
    ListMove {
        target: PrefabValueTarget,
        from: u32,
        to: u32,
    },
    MapInsert {
        target: PrefabValueTarget,
        key: ReflectedValueEnvelope,
        value: ReflectedValueEnvelope,
    },
    MapRemove {
        target: PrefabValueTarget,
        key: ReflectedValueEnvelope,
    },
    SetVariant {
        target: PrefabValueTarget,
        variant_name: String,
        value: Option<ReflectedValueEnvelope>,
    },
    AddComponent {
        entity_alias: String,
        component_type_path: String,
        initial_value: Option<ReflectedValueEnvelope>,
    },
    RemoveComponent {
        entity_alias: String,
        component_type_path: String,
    },
    AddEntity {
        alias: String,
        parent_alias: Option<String>,
    },
    RemoveEntity {
        alias: String,
    },
    ReparentEntity {
        alias: String,
        parent_alias: Option<String>,
    },
    AddInstance {
        alias: String,
        source_asset: String,
        parent_entity_alias: Option<String>,
    },
    RemoveInstance {
        alias: String,
    },
    ReparentInstance {
        alias: String,
        parent_entity_alias: Option<String>,
    },
    SetOverride {
        target: PrefabValueTarget,
        value: ReflectedValueEnvelope,
    },
    ClearOverride {
        target: PrefabValueTarget,
    },
    InsertOverride {
        target: PrefabValueTarget,
        index: u32,
        value: ReflectedValueEnvelope,
    },
    RemoveOverrideItem {
        target: PrefabValueTarget,
        index: u32,
    },
    MoveOverride {
        target: PrefabValueTarget,
        from: u32,
        to: u32,
    },
    RemoveOverride {
        target: PrefabValueTarget,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabDiagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub target: Option<PrefabValueTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefabRpcResult {
    pub snapshot: Option<PrefabSourceSnapshot>,
    pub diagnostics: Vec<PrefabDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypedActionResult {
    pub snapshot: Option<PrefabSourceSnapshot>,
    pub changed_paths: Vec<ReflectedPath>,
    pub diagnostics: Vec<PrefabDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSessionCommand {
    Open,
    Save,
    SaveRecovery,
    Undo,
    Redo,
    Close,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSessionStatus {
    pub open: bool,
    pub revision: u64,
    pub dirty: bool,
    pub undo_depth: u32,
    pub redo_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSessionResult {
    pub status: SourceSessionStatus,
    pub snapshot: Option<PrefabSourceSnapshot>,
    pub diagnostics: Vec<PrefabDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceAuthoringSessionCommand {
    Open,
    Apply(SourceFileEditOperation),
    Undo,
    Redo,
    Close,
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthoringSessionRequest {
    pub capability: Capability,
    pub session_id: String,
    pub source: WorkspaceSourceFileRef,
    pub expected_revision: u64,
    pub command: SourceAuthoringSessionCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceAuthoringSessionStatus {
    pub open: bool,
    pub revision: u64,
    pub undo_depth: u32,
    pub redo_depth: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceAuthoringSessionOutcome {
    Snapshot(SourceFileEditSnapshot),
    Closed,
    Failure(SourceAuthoringFailure),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAuthoringFailureCode {
    Unavailable,
    NotOpen,
    RevisionConflict,
    HistoryEmpty,
    Transaction,
    SourceMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthoringFailure {
    pub code: SourceAuthoringFailureCode,
    pub detail: String,
    pub expected_revision: u64,
    pub current_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAuthoringSessionResult {
    pub status: SourceAuthoringSessionStatus,
    pub outcome: SourceAuthoringSessionOutcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectorParityCase {
    pub id: &'static str,
    pub renderer_or_behavior: &'static str,
    pub golden_fixture: &'static str,
}

pub const INSPECTOR_PARITY_MATRIX: &[InspectorParityCase] = &[
    parity(
        "scalar_editing",
        "scalar editing",
        "inspector-behaviors.ron",
    ),
    parity("slider_range", "slider and range", "mesh.inspector.ron"),
    parity(
        "multiline_text",
        "multiline text",
        "inspector-behaviors.ron",
    ),
    parity(
        "color_vector",
        "color and vector",
        "transform.inspector.ron",
    ),
    parity(
        "enum_variant",
        "enum variant switching",
        "camera.inspector.ron",
    ),
    parity(
        "nested_struct",
        "nested struct editing",
        "camera.inspector.ron",
    ),
    parity(
        "list_editing",
        "list editing",
        "material-assignment.inspector.ron",
    ),
    parity(
        "map_editing",
        "map editing",
        "nested-collections.inspector.ron",
    ),
    parity(
        "typed_map_keys",
        "typed map keys",
        "nested-collections.inspector.ron",
    ),
    parity(
        "asset_object_refs",
        "asset and object references",
        "asset-handle.inspector.ron",
    ),
    parity(
        "mixed_selection",
        "mixed multi-selection",
        "inspector-behaviors.ron",
    ),
    parity("undo_redo", "undo and redo", "inspector-behaviors.ron"),
    parity(
        "visibility_read_only",
        "visibility and read-only rules",
        "inspector-behaviors.ron",
    ),
    parity(
        "validation",
        "validation diagnostics",
        "camera.inspector.ron",
    ),
    parity("actions", "action invocation", "inspector-behaviors.ron"),
    parity(
        "add_component",
        "Add Component search, applicability, and defaults",
        "add-component.inspector.ron",
    ),
    parity(
        "gamedata",
        "GameData columns and rows",
        "gamedata-row.inspector.ron",
    ),
    parity(
        "graph_ports",
        "graph port descriptors and defaults",
        "graph-port.inspector.ron",
    ),
];

const fn parity(
    id: &'static str,
    renderer_or_behavior: &'static str,
    golden_fixture: &'static str,
) -> InspectorParityCase {
    InspectorParityCase {
        id,
        renderer_or_behavior,
        golden_fixture,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn supported_contract_contains_the_vnext_surface() {
        const { assert!(SUPPORTED_RPC_ROUTING_ENABLED) };
        assert!(CAPNP_SCHEMA.contains("struct TypeRegistrySnapshot"));
        assert!(CAPNP_SCHEMA.contains("struct PrefabEditCommand"));
        assert!(CAPNP_SCHEMA.contains("typeRegistrySnapshot @11"));
        assert!(CAPNP_SCHEMA.contains("sourceSessionLifecycle @16"));
        assert!(CAPNP_SCHEMA.contains("applicability @7 :ApplicabilityDescriptor"));
        assert!(CAPNP_SCHEMA.contains("reflectedDefault @8 :Authoring.ReflectedValueEnvelope"));
        assert!(CAPNP_SCHEMA.contains("constraints @9 :FieldConstraints"));
        assert!(CAPNP_SCHEMA.contains("clearOverride @17 :RemoveOverride"));
        assert!(CAPNP_SCHEMA.contains("moveOverride @20 :ListMove"));
    }

    #[test]
    fn inspector_parity_matrix_has_unique_acceptance_cases() {
        let ids = INSPECTOR_PARITY_MATRIX
            .iter()
            .map(|case| case.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), INSPECTOR_PARITY_MATRIX.len());
        assert_eq!(INSPECTOR_PARITY_MATRIX.len(), 18);
        assert!(
            INSPECTOR_PARITY_MATRIX
                .iter()
                .all(|case| !case.golden_fixture.is_empty())
        );
    }
}
