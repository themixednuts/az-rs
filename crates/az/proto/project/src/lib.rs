//! Cap'n Proto protocol types for `project-host`.

pub mod vnext;
mod vnext_wire;

// Machine-generated Cap'n Proto output: written into OUT_DIR by build.rs and
// regenerated on every build, so it is not in git and has no per-site fix. The
// macro completes upstream's own `#![allow(clippy::all)]` for the pedantic and
// nursery groups this workspace denies.
az_proto_core::generated_schema!(pub mod project_capnp, "azoth/project_capnp.rs");

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::Path;

use az_node_graph::{
    GeneratedRustGraphAbi, GraphCommand, GraphComment, GraphCommentBounds, GraphCommentId,
    GraphCompilerBackendDescriptor, GraphCompilerBackendKind, GraphConnection, GraphConnectionId,
    GraphConnectionRoute, GraphDocumentTemplate, GraphExecutionMode, GraphNode,
    GraphNodeCatalogRequirement, GraphNodeId, GraphNodeLayout, GraphPalettePolicy, GraphPoint,
    GraphPortRef, GraphRouteAnchor, GraphRouteAnchorId, GraphRouteAnchorKind,
    GraphRouteSegmentConstraint, GraphRouteStyle, GraphSourceWorkflow, GraphSourceWorkflowKind,
    GraphTypeCatalog, GraphTypeDescriptor, GraphTypeId, NodeCapability, NodePortAttachment,
    NodePortCapacity, NodePortDescriptor, NodePortDirection, NodePortId, NodePortLayout,
    NodePortSide, NodePortValue, NodeRuntimeBinding, NodeSourceLink, NodeTypeCatalog,
    NodeTypeDescriptor, NodeTypeId, RuntimeGraphExecutionStrategy, RuntimeGraphProductDescriptor,
    RustCallResult, RustDataflowOutput, RustDataflowOutputField, RustDataflowParameter,
    RustDataflowParameterSource, RustNodeCallAbi, RustTypedDataflowNodeCall, RustValuePassing,
    VisualGraphDocument,
};
use az_proto_asset::WorkspaceSourceFileRef;
use az_proto_core::{Capability, SideChannelHandle, StagingFileSideChannelError};
use az_proto_runtime::{
    RUNTIME_CONTROL_PERMISSION, RUNTIME_HOST_AUDIENCE, RuntimeAssetPackageRoot,
    RuntimeAssetSourceRoot, RuntimeRole, validate_runtime_asset_package_roots,
};
use capnp::Error;

use self::vnext_wire::ReflectedValueEnvelopeCapnp as _;
use capnp::{message, serialize_packed};
use thiserror::Error;
use uuid::Uuid;

pub use az_proto_core as core;

pub const GAMEDATA_CATALOG_SNAPSHOT_VERSION: u32 = 1;
pub const NODE_TYPE_CATALOG_SNAPSHOT_VERSION: u32 = 1;
pub const GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION: u32 = 1;
pub const GRAPH_COMMAND_BATCH_SNAPSHOT_VERSION: u32 = 1;
pub const GRAPH_COMMAND_STATUS_SNAPSHOT_VERSION: u32 = 1;
pub const GRAPH_DOCUMENT_SNAPSHOT_VERSION: u32 = 1;
pub const PROJECT_HOST_NAMESPACE: &str = "azoth";
pub const PROJECT_HOST_SERVICE_NAME: &str = "project-host";
pub const PROJECT_HOST_AUDIENCE: &str = "project-host";
pub const PROJECT_SCHEMA_PERMISSION: &str = "project.schema";
pub const PROJECT_GAMEDATA_PERMISSION: &str = "project.gamedata";
pub const PROJECT_NODE_CATALOG_PERMISSION: &str = "project.node.catalog";
pub const PROJECT_GRAPH_CATALOG_PERMISSION: &str = "project.graph.catalog";
pub const PROJECT_INVENTORY_PERMISSION: &str = "project.inventory";
pub const PROJECT_EDIT_PERMISSION: &str = "project.edit";
pub const PROJECT_DOCUMENT_READ_PERMISSION: &str = "project.document.read";
pub const PROJECT_DOCUMENT_WRITE_PERMISSION: &str = "project.document.write";
pub const PROJECT_RUNTIME_LAUNCH_PERMISSION: &str = "project.runtime.launch";
pub const PROJECT_SOURCE_NAVIGATION_PERMISSION: &str = "project.source.navigation";

/// Converts a project protocol mirror into a generated Cap'n Proto builder.
pub trait ToCapnp<B> {
    /// # Errors
    ///
    /// Returns an error if the value fails its transport validation, if one of
    /// its lists is longer than a Cap'n Proto list can address, or if the
    /// message runs out of space while writing it.
    fn to_capnp(&self, builder: B) -> Result<(), Error>;
}

/// Converts a generated Cap'n Proto reader into a project protocol mirror.
pub trait FromCapnp<R>: Sized {
    /// # Errors
    ///
    /// Returns an error if a field of the value is absent from the message or
    /// is not valid UTF-8, or if the decoded value fails its transport
    /// validation.
    fn from_capnp(reader: R) -> Result<Self, Error>;
}

/// Project-relative identity used by graph document RPCs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentId(String);

impl DocumentId {
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn to_capnp(&self, builder: project_capnp::document_id::Builder<'_>) {
        write_document_id(self.as_str(), builder);
    }

    /// # Errors
    ///
    /// Returns an error if a field of the document id is absent from the message or is not valid
    /// UTF-8.
    pub fn from_capnp(reader: project_capnp::document_id::Reader<'_>) -> Result<Self, Error> {
        read_document_id(reader).map(Self::new)
    }
}

/// Monotonic revision used by graph document RPCs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DocumentRevision(pub u64);

impl DocumentRevision {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn to_capnp(self, builder: project_capnp::document_revision::Builder<'_>) {
        write_revision(self.0, builder);
    }

    #[must_use]
    pub fn from_capnp(reader: project_capnp::document_revision::Reader<'_>) -> Self {
        Self::new(read_revision(reader))
    }
}

fn write_document_id(value: &str, mut builder: project_capnp::document_id::Builder<'_>) {
    builder.set_value(value);
}

fn read_document_id(reader: project_capnp::document_id::Reader<'_>) -> Result<String, Error> {
    Ok(reader.get_value()?.to_string()?)
}

fn write_revision(value: u64, mut builder: project_capnp::document_revision::Builder<'_>) {
    builder.set_value(value);
}

fn read_revision(reader: project_capnp::document_revision::Reader<'_>) -> u64 {
    reader.get_value()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedDocument {
    pub document_id: DocumentId,
    pub revision: DocumentRevision,
    pub source_path: String,
    pub schema_type: String,
    pub content_hash: Vec<u8>,
    pub byte_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveDocumentResult {
    pub revision: DocumentRevision,
    pub saved: SavedDocument,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSideChannelResult {
    pub snapshot: SideChannelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataCatalogSnapshot {
    pub catalog_version: u32,
    pub generated_unix_ms: u64,
    pub tables: Vec<GameDataTableDescriptor>,
    pub families: Vec<GameDataTableFamilyDescriptor>,
    pub managers: Vec<GameDataManagerCatalogEntry>,
    pub diagnostics: Vec<GameDataCatalogDiagnostic>,
}

impl GameDataCatalogSnapshot {
    #[must_use]
    pub const fn new(
        catalog_version: u32,
        generated_unix_ms: u64,
        tables: Vec<GameDataTableDescriptor>,
        families: Vec<GameDataTableFamilyDescriptor>,
        managers: Vec<GameDataManagerCatalogEntry>,
        diagnostics: Vec<GameDataCatalogDiagnostic>,
    ) -> Self {
        Self {
            catalog_version,
            generated_unix_ms,
            tables,
            families,
            managers,
            diagnostics,
        }
    }

    #[must_use]
    pub const fn empty(generated_unix_ms: u64) -> Self {
        Self::new(
            GAMEDATA_CATALOG_SNAPSHOT_VERSION,
            generated_unix_ms,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }

    #[must_use]
    pub fn manager(&self, key: &str) -> Option<&GameDataManagerCatalogEntry> {
        self.managers.iter().find(|manager| manager.key == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataTableDescriptor {
    pub name: String,
    pub row_type: String,
    pub source_root: String,
    pub source_path: String,
    pub owner: String,
    pub schema_hash: Option<u64>,
    pub document_id: String,
    pub schema_type: String,
    pub category: String,
    pub row_count: Option<u64>,
    pub families: Vec<String>,
    /// Canonical portable source identity used by generic authoring sessions.
    pub source_ref: WorkspaceSourceFileRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataTableFamilyDescriptor {
    pub name: String,
    pub row_type: String,
    pub owner: String,
    pub duplicate_key_policy: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataProviderTarget {
    pub kind: String,
    pub name: String,
    pub row_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerNodeRef {
    pub key: String,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataKeyPolicy {
    pub kind: String,
    pub transforms: Vec<String>,
    pub reject_zero_crc: bool,
    pub store_key_text: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerInput {
    pub kind: String,
    pub name: String,
    pub row_type: String,
    pub source_root: String,
    pub source_path: String,
    pub detail: String,
    pub provider_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataRowFilter {
    pub field: String,
    pub predicate: String,
    pub compare_field: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataProjectionTransform {
    pub field: String,
    pub source_column: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataSecondaryIndex {
    pub name: String,
    pub field: String,
    pub key_kind: String,
    pub storage: String,
    pub duplicate_key_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataCatalogDiagnostic {
    pub code: String,
    pub message: String,
    pub target_key: String,
    pub target_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataManagerCatalogEntry {
    pub key: String,
    pub name: String,
    pub owner: String,
    pub row_type: String,
    pub kind: String,
    pub output_type: String,
    pub read_only: bool,
    pub provider_target: Option<GameDataProviderTarget>,
    pub key_policy: GameDataKeyPolicy,
    pub duplicate_key_policy: String,
    pub inputs: Vec<GameDataManagerInput>,
    pub row_filters: Vec<GameDataRowFilter>,
    pub projection_transforms: Vec<GameDataProjectionTransform>,
    pub secondary_indexes: Vec<GameDataSecondaryIndex>,
    pub source_targets: Vec<GameDataProviderTarget>,
    pub dependencies: Vec<GameDataManagerNodeRef>,
    pub dependents: Vec<GameDataManagerNodeRef>,
    pub diagnostics: Vec<GameDataCatalogDiagnostic>,
    pub projection_hash: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectInventoryGemKind {
    Project,
    Engine,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectInventoryLockState {
    Fresh,
    Missing,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInventoryLockStatus {
    pub state: ProjectInventoryLockState,
    pub path: String,
    pub diagnostic: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectInventoryRegistryCounts {
    pub build_rules: u64,
    pub node_types: u64,
    pub graph_types: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInventoryGem {
    pub id: String,
    pub expected_package: String,
    pub name: String,
    pub version: String,
    pub kind: ProjectInventoryGemKind,
    pub expected: bool,
    pub active: bool,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectInventoryReport {
    pub service_role: String,
    pub lock_status: ProjectInventoryLockStatus,
    pub gems: Vec<ProjectInventoryGem>,
    pub registry: ProjectInventoryRegistryCounts,
    pub diagnostics: Vec<String>,
    pub degraded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommandBatchRequest {
    pub capability: Capability,
    pub batch: SideChannelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGraphDocumentRequest {
    pub capability: Capability,
    pub document_id: DocumentId,
    pub graph_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSourceLinkRequest {
    pub capability: Capability,
    pub source_link: NodeSourceLink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSourceLinkPathKind {
    Unresolved,
    Absolute,
    PackageRelative,
    WorkspaceRelative,
    DocsOnly,
}

impl NodeSourceLinkPathKind {
    #[must_use]
    pub const fn to_capnp(self) -> project_capnp::NodeSourceLinkPathKind {
        match self {
            Self::Unresolved => project_capnp::NodeSourceLinkPathKind::Unresolved,
            Self::Absolute => project_capnp::NodeSourceLinkPathKind::Absolute,
            Self::PackageRelative => project_capnp::NodeSourceLinkPathKind::PackageRelative,
            Self::WorkspaceRelative => project_capnp::NodeSourceLinkPathKind::WorkspaceRelative,
            Self::DocsOnly => project_capnp::NodeSourceLinkPathKind::DocsOnly,
        }
    }

    #[must_use]
    pub const fn from_capnp(kind: project_capnp::NodeSourceLinkPathKind) -> Self {
        match kind {
            project_capnp::NodeSourceLinkPathKind::Unresolved => Self::Unresolved,
            project_capnp::NodeSourceLinkPathKind::Absolute => Self::Absolute,
            project_capnp::NodeSourceLinkPathKind::PackageRelative => Self::PackageRelative,
            project_capnp::NodeSourceLinkPathKind::WorkspaceRelative => Self::WorkspaceRelative,
            project_capnp::NodeSourceLinkPathKind::DocsOnly => Self::DocsOnly,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSourceLinkTarget {
    pub source_link: NodeSourceLink,
    pub resolved_path: Option<String>,
    pub package_id: Option<String>,
    pub package_root: Option<String>,
    pub path_kind: NodeSourceLinkPathKind,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphDocumentSnapshot {
    pub document_id: DocumentId,
    pub revision: DocumentRevision,
    pub document: VisualGraphDocument,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphCommandBatchSnapshot {
    pub document_id: DocumentId,
    pub expected_revision: Option<DocumentRevision>,
    pub client_batch_id: String,
    pub commands: Vec<GraphCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommandStatusSnapshot {
    pub document_id: DocumentId,
    pub client_batch_id: String,
    pub applied_command_count: u32,
    pub outcome: GraphCommandStatusOutcome,
    pub diagnostics: Vec<GraphCommandDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphCommandStatusOutcome {
    Accepted {
        revision: DocumentRevision,
    },
    Rejected {
        command_index: Option<u32>,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphCommandDiagnostic {
    pub command_index: Option<u32>,
    pub severity: GraphDiagnosticSeverity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphDiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeLaunchSnapshotRequest {
    pub capability: Capability,
    pub runtime_launch_capability: Capability,
    pub role: RuntimeRole,
    pub project_id: String,
    pub session_id: Uuid,
    pub session_slug: String,
    pub project_root: String,
    pub workspace_path: String,
    pub workspace_id: i64,
    pub include_unsaved_journal: bool,
    pub launch_profile: String,
    pub asset_source_roots: Vec<RuntimeAssetSourceRoot>,
    pub asset_package_roots: Vec<RuntimeAssetPackageRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectHostCapabilityRequest {
    pub capability: Capability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDocumentRequest {
    pub capability: Capability,
    pub document_id: DocumentId,
}

impl ProjectDocumentRequest {
    #[must_use]
    pub fn new(capability: Capability, document_id: impl Into<String>) -> Self {
        Self {
            capability,
            document_id: DocumentId::new(document_id),
        }
    }
}

fn write_gamedata_catalog_request(
    request: &ProjectHostCapabilityRequest,
    mut builder: project_capnp::project_host::gamedata_catalog_params::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "GameData catalog request",
        &request.capability,
        PROJECT_GAMEDATA_PERMISSION,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_gamedata_catalog_request(
    reader: project_capnp::project_host::gamedata_catalog_params::Reader<'_>,
) -> Result<ProjectHostCapabilityRequest, Error> {
    let request = ProjectHostCapabilityRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_project_host_capability_request_for_transport(
        "GameData catalog request",
        &request.capability,
        PROJECT_GAMEDATA_PERMISSION,
    )?;
    Ok(request)
}

fn write_gamedata_catalog_result(
    snapshot: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::gamedata_catalog_results::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "GameData catalog result",
        capability,
        PROJECT_GAMEDATA_PERMISSION,
    )?;
    let snapshot = snapshot.clone().with_capability(capability.clone());
    validate_project_side_channel_result_for_transport(
        "GameData catalog result",
        &snapshot,
        PROJECT_GAMEDATA_PERMISSION,
    )?;
    core::SideChannelHandle::to_capnp(&snapshot, builder.reborrow().init_snapshot())
}

fn read_gamedata_catalog_result(
    reader: project_capnp::project_host::gamedata_catalog_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    let result = ProjectSideChannelResult {
        snapshot: core::SideChannelHandle::from_capnp(reader.get_snapshot()?)?,
    };
    validate_project_side_channel_result_for_transport(
        "GameData catalog result",
        &result.snapshot,
        PROJECT_GAMEDATA_PERMISSION,
    )?;
    Ok(result)
}

fn read_gamedata_catalog_result_for_capability(
    reader: project_capnp::project_host::gamedata_catalog_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_gamedata_catalog_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "GameData catalog result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_node_type_catalog_request(
    request: &ProjectHostCapabilityRequest,
    mut builder: project_capnp::project_host::node_type_catalog_params::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "node type catalog request",
        &request.capability,
        PROJECT_NODE_CATALOG_PERMISSION,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_node_type_catalog_request(
    reader: project_capnp::project_host::node_type_catalog_params::Reader<'_>,
) -> Result<ProjectHostCapabilityRequest, Error> {
    let request = ProjectHostCapabilityRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_project_host_capability_request_for_transport(
        "node type catalog request",
        &request.capability,
        PROJECT_NODE_CATALOG_PERMISSION,
    )?;
    Ok(request)
}

fn write_node_type_catalog_result(
    snapshot: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::node_type_catalog_results::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "node type catalog result",
        capability,
        PROJECT_NODE_CATALOG_PERMISSION,
    )?;
    let snapshot = snapshot.clone().with_capability(capability.clone());
    validate_project_side_channel_result_for_transport(
        "node type catalog result",
        &snapshot,
        PROJECT_NODE_CATALOG_PERMISSION,
    )?;
    core::SideChannelHandle::to_capnp(&snapshot, builder.reborrow().init_snapshot())
}

fn read_node_type_catalog_result(
    reader: project_capnp::project_host::node_type_catalog_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    let result = ProjectSideChannelResult {
        snapshot: core::SideChannelHandle::from_capnp(reader.get_snapshot()?)?,
    };
    validate_project_side_channel_result_for_transport(
        "node type catalog result",
        &result.snapshot,
        PROJECT_NODE_CATALOG_PERMISSION,
    )?;
    Ok(result)
}

fn read_node_type_catalog_result_for_capability(
    reader: project_capnp::project_host::node_type_catalog_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_node_type_catalog_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "node type catalog result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_graph_type_catalog_request(
    request: &ProjectHostCapabilityRequest,
    mut builder: project_capnp::project_host::graph_type_catalog_params::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "graph type catalog request",
        &request.capability,
        PROJECT_GRAPH_CATALOG_PERMISSION,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_graph_type_catalog_request(
    reader: project_capnp::project_host::graph_type_catalog_params::Reader<'_>,
) -> Result<ProjectHostCapabilityRequest, Error> {
    let request = ProjectHostCapabilityRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_project_host_capability_request_for_transport(
        "graph type catalog request",
        &request.capability,
        PROJECT_GRAPH_CATALOG_PERMISSION,
    )?;
    Ok(request)
}

fn write_graph_type_catalog_result(
    snapshot: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::graph_type_catalog_results::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "graph type catalog result",
        capability,
        PROJECT_GRAPH_CATALOG_PERMISSION,
    )?;
    let snapshot = snapshot.clone().with_capability(capability.clone());
    validate_project_side_channel_result_for_transport(
        "graph type catalog result",
        &snapshot,
        PROJECT_GRAPH_CATALOG_PERMISSION,
    )?;
    core::SideChannelHandle::to_capnp(&snapshot, builder.reborrow().init_snapshot())
}

fn read_graph_type_catalog_result(
    reader: project_capnp::project_host::graph_type_catalog_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    let result = ProjectSideChannelResult {
        snapshot: core::SideChannelHandle::from_capnp(reader.get_snapshot()?)?,
    };
    validate_project_side_channel_result_for_transport(
        "graph type catalog result",
        &result.snapshot,
        PROJECT_GRAPH_CATALOG_PERMISSION,
    )?;
    Ok(result)
}

fn read_graph_type_catalog_result_for_capability(
    reader: project_capnp::project_host::graph_type_catalog_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_graph_type_catalog_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "graph type catalog result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_project_inventory_request(
    request: &ProjectHostCapabilityRequest,
    mut builder: project_capnp::project_host::project_inventory_params::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "project inventory request",
        &request.capability,
        PROJECT_INVENTORY_PERMISSION,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())
}

fn read_project_inventory_request(
    reader: project_capnp::project_host::project_inventory_params::Reader<'_>,
) -> Result<ProjectHostCapabilityRequest, Error> {
    let request = ProjectHostCapabilityRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
    };
    validate_project_host_capability_request_for_transport(
        "project inventory request",
        &request.capability,
        PROJECT_INVENTORY_PERMISSION,
    )?;
    Ok(request)
}

fn write_project_inventory_result(
    report: &ProjectInventoryReport,
    capability: &Capability,
    mut builder: project_capnp::project_host::project_inventory_results::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "project inventory result",
        capability,
        PROJECT_INVENTORY_PERMISSION,
    )?;
    validate_project_inventory_report_for_transport(report)?;
    write_project_inventory_report(report, builder.reborrow().init_report())
}

fn read_project_inventory_result(
    reader: project_capnp::project_host::project_inventory_results::Reader<'_>,
) -> Result<ProjectInventoryReport, Error> {
    let report = read_project_inventory_report(reader.get_report()?)?;
    validate_project_inventory_report_for_transport(&report)?;
    Ok(report)
}

fn write_node_source_link_request(
    request: &NodeSourceLinkRequest,
    mut builder: project_capnp::project_host::resolve_node_source_link_params::Builder<'_>,
) -> Result<(), Error> {
    validate_node_source_link_request_for_transport(request)?;
    let mut request_builder = builder.reborrow().init_request();
    core::Capability::to_capnp(
        &request.capability,
        request_builder.reborrow().init_capability(),
    )?;
    write_node_source_link(
        &request.source_link,
        request_builder.reborrow().init_source_link(),
    );
    Ok(())
}

fn read_node_source_link_request(
    reader: project_capnp::project_host::resolve_node_source_link_params::Reader<'_>,
) -> Result<NodeSourceLinkRequest, Error> {
    let request_reader = reader.get_request()?;
    let request = NodeSourceLinkRequest {
        capability: core::Capability::from_capnp(request_reader.get_capability()?)?,
        source_link: read_node_source_link(request_reader.get_source_link()?)?,
    };
    validate_node_source_link_request_for_transport(&request)?;
    Ok(request)
}

fn write_node_source_link_target(
    target: &NodeSourceLinkTarget,
    mut builder: project_capnp::project_host::resolve_node_source_link_results::Builder<'_>,
) -> Result<(), Error> {
    validate_node_source_link_target_for_transport(target)?;
    let mut target_builder = builder.reborrow().init_target();
    if let Some(package) = &target.source_link.package {
        target_builder.set_package(package);
    }
    if let Some(module_path) = &target.source_link.module_path {
        target_builder.set_module_path(module_path);
    }
    if let Some(symbol_path) = &target.source_link.symbol_path {
        target_builder.set_symbol_path(symbol_path);
    }
    if let Some(file) = &target.source_link.file {
        target_builder.set_file(file);
    }
    if let Some(line) = target.source_link.line {
        target_builder.set_line(line);
    }
    if let Some(column) = target.source_link.column {
        target_builder.set_column(column);
    }
    if let Some(docs_url) = &target.source_link.docs_url {
        target_builder.set_docs_url(docs_url);
    }
    if let Some(resolved_path) = &target.resolved_path {
        target_builder.set_resolved_path(resolved_path);
    }
    if let Some(package_id) = &target.package_id {
        target_builder.set_package_id(package_id);
    }
    if let Some(package_root) = &target.package_root {
        target_builder.set_package_root(package_root);
    }
    target_builder.set_path_kind(target.path_kind.to_capnp());
    target_builder.set_exists(target.exists);
    Ok(())
}

fn read_node_source_link_target(
    reader: project_capnp::project_host::resolve_node_source_link_results::Reader<'_>,
) -> Result<NodeSourceLinkTarget, Error> {
    let target_reader = reader.get_target()?;
    let target = NodeSourceLinkTarget {
        source_link: NodeSourceLink {
            package: optional_text(target_reader.has_package(), target_reader.get_package())?,
            module_path: optional_text(
                target_reader.has_module_path(),
                target_reader.get_module_path(),
            )?,
            symbol_path: optional_text(
                target_reader.has_symbol_path(),
                target_reader.get_symbol_path(),
            )?,
            file: optional_text(target_reader.has_file(), target_reader.get_file())?,
            line: non_zero_u32_option(target_reader.get_line()),
            column: non_zero_u32_option(target_reader.get_column()),
            docs_url: optional_text(target_reader.has_docs_url(), target_reader.get_docs_url())?,
        },
        resolved_path: optional_text(
            target_reader.has_resolved_path(),
            target_reader.get_resolved_path(),
        )?,
        package_id: optional_text(
            target_reader.has_package_id(),
            target_reader.get_package_id(),
        )?,
        package_root: optional_text(
            target_reader.has_package_root(),
            target_reader.get_package_root(),
        )?,
        path_kind: target_reader
            .get_path_kind()
            .map(NodeSourceLinkPathKind::from_capnp)?,
        exists: target_reader.get_exists(),
    };
    validate_node_source_link_target_for_transport(&target)?;
    Ok(target)
}

fn write_apply_graph_commands_request(
    request: &GraphCommandBatchRequest,
    mut builder: project_capnp::project_host::apply_graph_commands_params::Builder<'_>,
) -> Result<(), Error> {
    let request = GraphCommandBatchRequest {
        capability: request.capability.clone(),
        batch: request
            .batch
            .clone()
            .with_capability(request.capability.clone()),
    };
    validate_graph_command_batch_request_for_transport("apply graph commands request", &request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    core::SideChannelHandle::to_capnp(&request.batch, builder.reborrow().init_batch())
}

fn read_apply_graph_commands_request(
    reader: project_capnp::project_host::apply_graph_commands_params::Reader<'_>,
) -> Result<GraphCommandBatchRequest, Error> {
    let request = GraphCommandBatchRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        batch: core::SideChannelHandle::from_capnp(reader.get_batch()?)?,
    };
    validate_graph_command_batch_request_for_transport("apply graph commands request", &request)?;
    Ok(request)
}

fn write_apply_graph_commands_result(
    status: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::apply_graph_commands_results::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "apply graph commands result",
        capability,
        PROJECT_EDIT_PERMISSION,
    )?;
    let status = status.clone().with_capability(capability.clone());
    validate_project_side_channel_result_for_transport(
        "apply graph commands result",
        &status,
        PROJECT_EDIT_PERMISSION,
    )?;
    core::SideChannelHandle::to_capnp(&status, builder.reborrow().init_status())
}

fn read_apply_graph_commands_result(
    reader: project_capnp::project_host::apply_graph_commands_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    let result = ProjectSideChannelResult {
        snapshot: core::SideChannelHandle::from_capnp(reader.get_status()?)?,
    };
    validate_project_side_channel_result_for_transport(
        "apply graph commands result",
        &result.snapshot,
        PROJECT_EDIT_PERMISSION,
    )?;
    Ok(result)
}

fn read_apply_graph_commands_result_for_capability(
    reader: project_capnp::project_host::apply_graph_commands_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_apply_graph_commands_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "apply graph commands result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_create_graph_document_request(
    request: &CreateGraphDocumentRequest,
    mut builder: project_capnp::project_host::create_graph_document_params::Builder<'_>,
) -> Result<(), Error> {
    validate_create_graph_document_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    write_document_id(
        request.document_id.as_str(),
        builder.reborrow().init_document(),
    );
    builder.set_graph_type(&request.graph_type);
    Ok(())
}

fn read_create_graph_document_request(
    reader: project_capnp::project_host::create_graph_document_params::Reader<'_>,
) -> Result<CreateGraphDocumentRequest, Error> {
    let request = CreateGraphDocumentRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        document_id: DocumentId::new(read_document_id(reader.get_document()?)?),
        graph_type: reader.get_graph_type()?.to_string()?,
    };
    validate_create_graph_document_request_for_transport(&request)?;
    Ok(request)
}

fn write_create_graph_document_result(
    snapshot: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::create_graph_document_results::Builder<'_>,
) -> Result<(), Error> {
    write_graph_document_side_channel_result(
        "create graph document result",
        PROJECT_DOCUMENT_WRITE_PERMISSION,
        snapshot,
        capability,
        builder.reborrow().init_snapshot(),
    )
}

fn read_create_graph_document_result(
    reader: project_capnp::project_host::create_graph_document_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    read_graph_document_side_channel_result(
        "create graph document result",
        PROJECT_DOCUMENT_WRITE_PERMISSION,
        reader.get_snapshot()?,
    )
}

fn read_create_graph_document_result_for_capability(
    reader: project_capnp::project_host::create_graph_document_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_create_graph_document_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "create graph document result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_graph_document_snapshot_request(
    request: &ProjectDocumentRequest,
    mut builder: project_capnp::project_host::graph_document_snapshot_params::Builder<'_>,
) -> Result<(), Error> {
    validate_project_document_request_for_transport(
        "graph document snapshot request",
        PROJECT_DOCUMENT_READ_PERMISSION,
        request,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    write_document_id(request.document_id.as_str(), builder.init_document());
    Ok(())
}

fn read_graph_document_snapshot_request(
    reader: project_capnp::project_host::graph_document_snapshot_params::Reader<'_>,
) -> Result<ProjectDocumentRequest, Error> {
    read_project_document_request_fields(
        "graph document snapshot request",
        PROJECT_DOCUMENT_READ_PERMISSION,
        reader.get_capability()?,
        reader.get_document()?,
    )
}

fn write_graph_document_snapshot_result(
    snapshot: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::graph_document_snapshot_results::Builder<'_>,
) -> Result<(), Error> {
    write_graph_document_side_channel_result(
        "graph document snapshot result",
        PROJECT_DOCUMENT_READ_PERMISSION,
        snapshot,
        capability,
        builder.reborrow().init_snapshot(),
    )
}

fn read_graph_document_snapshot_result(
    reader: project_capnp::project_host::graph_document_snapshot_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    read_graph_document_side_channel_result(
        "graph document snapshot result",
        PROJECT_DOCUMENT_READ_PERMISSION,
        reader.get_snapshot()?,
    )
}

fn read_graph_document_snapshot_result_for_capability(
    reader: project_capnp::project_host::graph_document_snapshot_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_graph_document_snapshot_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "graph document snapshot result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_save_graph_document_request(
    request: &ProjectDocumentRequest,
    mut builder: project_capnp::project_host::save_graph_document_params::Builder<'_>,
) -> Result<(), Error> {
    validate_project_document_request_for_transport(
        "save graph document request",
        PROJECT_DOCUMENT_WRITE_PERMISSION,
        request,
    )?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    write_document_id(request.document_id.as_str(), builder.init_document());
    Ok(())
}

fn read_save_graph_document_request(
    reader: project_capnp::project_host::save_graph_document_params::Reader<'_>,
) -> Result<ProjectDocumentRequest, Error> {
    read_project_document_request_fields(
        "save graph document request",
        PROJECT_DOCUMENT_WRITE_PERMISSION,
        reader.get_capability()?,
        reader.get_document()?,
    )
}

fn write_save_graph_document_result(
    result: &SaveDocumentResult,
    mut builder: project_capnp::project_host::save_graph_document_results::Builder<'_>,
) -> Result<(), Error> {
    validate_save_document_result_for_transport(result)?;
    write_revision(result.revision.0, builder.reborrow().init_revision());
    write_saved_document(&result.saved, builder.init_saved())
}

fn read_save_graph_document_result(
    reader: project_capnp::project_host::save_graph_document_results::Reader<'_>,
) -> Result<SaveDocumentResult, Error> {
    let result = SaveDocumentResult {
        revision: DocumentRevision::new(read_revision(reader.get_revision()?)),
        saved: read_saved_document(reader.get_saved()?)?,
    };
    validate_save_document_result_for_transport(&result)?;
    Ok(result)
}

fn write_graph_document_side_channel_result(
    kind: &'static str,
    permission: &'static str,
    snapshot: &SideChannelHandle,
    capability: &Capability,
    builder: az_proto_core::core_capnp::side_channel_handle::Builder<'_>,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(kind, capability, permission)?;
    let snapshot = snapshot.clone().with_capability(capability.clone());
    validate_project_side_channel_result_for_transport(kind, &snapshot, permission)?;
    core::SideChannelHandle::to_capnp(&snapshot, builder)
}

fn read_graph_document_side_channel_result(
    kind: &'static str,
    permission: &'static str,
    reader: az_proto_core::core_capnp::side_channel_handle::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    let result = ProjectSideChannelResult {
        snapshot: core::SideChannelHandle::from_capnp(reader)?,
    };
    validate_project_side_channel_result_for_transport(kind, &result.snapshot, permission)?;
    Ok(result)
}

fn write_runtime_launch_snapshot_result(
    snapshot: &SideChannelHandle,
    capability: &Capability,
    mut builder: project_capnp::project_host::runtime_launch_snapshot_results::Builder<'_>,
) -> Result<(), Error> {
    validate_project_runtime_launch_side_channel_capability(
        "runtime launch snapshot result",
        capability,
    )?;
    let snapshot = snapshot.clone().with_capability(capability.clone());
    validate_runtime_launch_side_channel_result_for_transport(
        "runtime launch snapshot result",
        &snapshot,
    )?;
    core::SideChannelHandle::to_capnp(&snapshot, builder.reborrow().init_snapshot())
}

fn read_runtime_launch_snapshot_result(
    reader: project_capnp::project_host::runtime_launch_snapshot_results::Reader<'_>,
) -> Result<ProjectSideChannelResult, Error> {
    let result = ProjectSideChannelResult {
        snapshot: core::SideChannelHandle::from_capnp(reader.get_snapshot()?)?,
    };
    validate_runtime_launch_side_channel_result_for_transport(
        "runtime launch snapshot result",
        &result.snapshot,
    )?;
    Ok(result)
}

fn read_runtime_launch_snapshot_result_for_capability(
    reader: project_capnp::project_host::runtime_launch_snapshot_results::Reader<'_>,
    expected: &Capability,
) -> Result<ProjectSideChannelResult, Error> {
    let result = read_runtime_launch_snapshot_result(reader)?;
    validate_project_side_channel_result_matches_capability(
        "runtime launch snapshot result",
        &result.snapshot,
        expected,
    )?;
    Ok(result)
}

fn write_saved_document(
    saved: &SavedDocument,
    mut builder: project_capnp::saved_document::Builder<'_>,
) -> Result<(), Error> {
    validate_saved_document_for_transport(saved)?;
    write_document_id(
        saved.document_id.as_str(),
        builder.reborrow().init_document(),
    );
    write_revision(saved.revision.0, builder.reborrow().init_revision());
    builder.set_source_path(&saved.source_path);
    builder.set_schema_type(&saved.schema_type);
    builder.set_content_hash(&saved.content_hash);
    builder.set_byte_length(saved.byte_length);
    Ok(())
}

fn read_saved_document(
    reader: project_capnp::saved_document::Reader<'_>,
) -> Result<SavedDocument, Error> {
    let saved = SavedDocument {
        document_id: DocumentId::new(read_document_id(reader.get_document()?)?),
        revision: DocumentRevision::new(read_revision(reader.get_revision()?)),
        source_path: reader.get_source_path()?.to_string()?,
        schema_type: reader.get_schema_type()?.to_string()?,
        content_hash: reader.get_content_hash()?.to_vec(),
        byte_length: reader.get_byte_length(),
    };
    validate_saved_document_for_transport(&saved)?;
    Ok(saved)
}

fn write_runtime_launch_snapshot_request(
    request: &RuntimeLaunchSnapshotRequest,
    mut builder: project_capnp::runtime_launch_snapshot_request::Builder<'_>,
) -> Result<(), Error> {
    validate_runtime_launch_snapshot_request_for_transport(request)?;
    core::Capability::to_capnp(&request.capability, builder.reborrow().init_capability())?;
    core::Capability::to_capnp(
        &request.runtime_launch_capability,
        builder.reborrow().init_runtime_launch_capability(),
    )?;
    builder.set_role(request.role.to_capnp());
    builder.set_project_id(&request.project_id);
    core::SessionId::from(request.session_id).to_capnp(builder.reborrow().init_session_id());
    builder.set_session_slug(&request.session_slug);
    builder.set_project_root(&request.project_root);
    builder.set_workspace_path(&request.workspace_path);
    builder.set_workspace_id(request.workspace_id);
    builder.set_include_unsaved_journal(request.include_unsaved_journal);
    builder.set_launch_profile(&request.launch_profile);
    let mut source_roots = builder
        .reborrow()
        .init_asset_source_roots(capnp_list_index(request.asset_source_roots.len())?);
    for (index, source_root) in request.asset_source_roots.iter().enumerate() {
        (source_root).to_capnp(source_roots.reborrow().get(capnp_list_index(index)?));
    }
    let mut package_roots = builder
        .reborrow()
        .init_asset_package_roots(capnp_list_index(request.asset_package_roots.len())?);
    for (index, package_root) in request.asset_package_roots.iter().enumerate() {
        (package_root).to_capnp(package_roots.reborrow().get(capnp_list_index(index)?));
    }
    Ok(())
}

fn read_runtime_launch_snapshot_request(
    reader: project_capnp::runtime_launch_snapshot_request::Reader<'_>,
) -> Result<RuntimeLaunchSnapshotRequest, Error> {
    let role = reader
        .get_role()
        .map(RuntimeRole::from_capnp)
        .map_err(|error| Error::failed(format!("unknown runtime role: {error:?}")))?;
    let request = RuntimeLaunchSnapshotRequest {
        capability: core::Capability::from_capnp(reader.get_capability()?)?,
        runtime_launch_capability: core::Capability::from_capnp(
            reader.get_runtime_launch_capability()?,
        )?,
        role,
        project_id: reader.get_project_id()?.to_string()?,
        session_id: core::SessionId::from_capnp(reader.get_session_id()?)
            .map(core::SessionId::into_uuid)?,
        session_slug: reader.get_session_slug()?.to_string()?,
        project_root: reader.get_project_root()?.to_string()?,
        workspace_path: reader.get_workspace_path()?.to_string()?,
        workspace_id: reader.get_workspace_id(),
        include_unsaved_journal: reader.get_include_unsaved_journal(),
        launch_profile: reader.get_launch_profile()?.to_string()?,
        asset_source_roots: reader
            .get_asset_source_roots()?
            .iter()
            .map(RuntimeAssetSourceRoot::from_capnp)
            .collect::<Result<Vec<_>, Error>>()?,
        asset_package_roots: reader
            .get_asset_package_roots()?
            .iter()
            .map(RuntimeAssetPackageRoot::from_capnp)
            .collect::<Result<Vec<_>, Error>>()?,
    };
    validate_runtime_launch_snapshot_request_for_transport(&request)?;
    Ok(request)
}

impl<'a> ToCapnp<project_capnp::project_host::gamedata_catalog_params::Builder<'a>>
    for ProjectHostCapabilityRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::gamedata_catalog_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_gamedata_catalog_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::gamedata_catalog_params::Reader<'a>>
    for ProjectHostCapabilityRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::gamedata_catalog_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_gamedata_catalog_request(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::node_type_catalog_params::Builder<'a>>
    for ProjectHostCapabilityRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::node_type_catalog_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_node_type_catalog_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::node_type_catalog_params::Reader<'a>>
    for ProjectHostCapabilityRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::node_type_catalog_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_node_type_catalog_request(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::graph_type_catalog_params::Builder<'a>>
    for ProjectHostCapabilityRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::graph_type_catalog_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_graph_type_catalog_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::graph_type_catalog_params::Reader<'a>>
    for ProjectHostCapabilityRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::graph_type_catalog_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_graph_type_catalog_request(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::project_inventory_params::Builder<'a>>
    for ProjectHostCapabilityRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::project_inventory_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_project_inventory_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::project_inventory_params::Reader<'a>>
    for ProjectHostCapabilityRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::project_inventory_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_project_inventory_request(reader)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::gamedata_catalog_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::gamedata_catalog_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_gamedata_catalog_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::gamedata_catalog_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::gamedata_catalog_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_gamedata_catalog_result_for_capability(reader, capability)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::node_type_catalog_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::node_type_catalog_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_node_type_catalog_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::node_type_catalog_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::node_type_catalog_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_node_type_catalog_result_for_capability(reader, capability)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::graph_type_catalog_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::graph_type_catalog_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_graph_type_catalog_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::graph_type_catalog_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::graph_type_catalog_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_graph_type_catalog_result_for_capability(reader, capability)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::project_inventory_results::Builder<'a>,
        &'b Capability,
    )> for ProjectInventoryReport
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::project_inventory_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_project_inventory_result(self, capability, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::project_inventory_results::Reader<'a>>
    for ProjectInventoryReport
{
    fn from_capnp(
        reader: project_capnp::project_host::project_inventory_results::Reader<'a>,
    ) -> Result<Self, Error> {
        read_project_inventory_result(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::resolve_node_source_link_params::Builder<'a>>
    for NodeSourceLinkRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::resolve_node_source_link_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_node_source_link_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::resolve_node_source_link_params::Reader<'a>>
    for NodeSourceLinkRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::resolve_node_source_link_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_node_source_link_request(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::resolve_node_source_link_results::Builder<'a>>
    for NodeSourceLinkTarget
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::resolve_node_source_link_results::Builder<'a>,
    ) -> Result<(), Error> {
        write_node_source_link_target(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::resolve_node_source_link_results::Reader<'a>>
    for NodeSourceLinkTarget
{
    fn from_capnp(
        reader: project_capnp::project_host::resolve_node_source_link_results::Reader<'a>,
    ) -> Result<Self, Error> {
        read_node_source_link_target(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::apply_graph_commands_params::Builder<'a>>
    for GraphCommandBatchRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::apply_graph_commands_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_apply_graph_commands_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::apply_graph_commands_params::Reader<'a>>
    for GraphCommandBatchRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::apply_graph_commands_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_apply_graph_commands_request(reader)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::apply_graph_commands_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::apply_graph_commands_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_apply_graph_commands_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::apply_graph_commands_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::apply_graph_commands_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_apply_graph_commands_result_for_capability(reader, capability)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::create_graph_document_params::Builder<'a>>
    for CreateGraphDocumentRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::create_graph_document_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_create_graph_document_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::create_graph_document_params::Reader<'a>>
    for CreateGraphDocumentRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::create_graph_document_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_create_graph_document_request(reader)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::create_graph_document_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::create_graph_document_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_create_graph_document_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::create_graph_document_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::create_graph_document_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_create_graph_document_result_for_capability(reader, capability)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::graph_document_snapshot_params::Builder<'a>>
    for ProjectDocumentRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::graph_document_snapshot_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_graph_document_snapshot_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::graph_document_snapshot_params::Reader<'a>>
    for ProjectDocumentRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::graph_document_snapshot_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_graph_document_snapshot_request(reader)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::graph_document_snapshot_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::graph_document_snapshot_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_graph_document_snapshot_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::graph_document_snapshot_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::graph_document_snapshot_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_graph_document_snapshot_result_for_capability(reader, capability)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::save_graph_document_params::Builder<'a>>
    for ProjectDocumentRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::save_graph_document_params::Builder<'a>,
    ) -> Result<(), Error> {
        write_save_graph_document_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::save_graph_document_params::Reader<'a>>
    for ProjectDocumentRequest
{
    fn from_capnp(
        reader: project_capnp::project_host::save_graph_document_params::Reader<'a>,
    ) -> Result<Self, Error> {
        read_save_graph_document_request(reader)
    }
}

impl<'a> ToCapnp<project_capnp::project_host::save_graph_document_results::Builder<'a>>
    for SaveDocumentResult
{
    fn to_capnp(
        &self,
        builder: project_capnp::project_host::save_graph_document_results::Builder<'a>,
    ) -> Result<(), Error> {
        write_save_graph_document_result(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::project_host::save_graph_document_results::Reader<'a>>
    for SaveDocumentResult
{
    fn from_capnp(
        reader: project_capnp::project_host::save_graph_document_results::Reader<'a>,
    ) -> Result<Self, Error> {
        read_save_graph_document_result(reader)
    }
}

impl<'a, 'b>
    ToCapnp<(
        project_capnp::project_host::runtime_launch_snapshot_results::Builder<'a>,
        &'b Capability,
    )> for SideChannelHandle
{
    fn to_capnp(
        &self,
        (builder, capability): (
            project_capnp::project_host::runtime_launch_snapshot_results::Builder<'a>,
            &'b Capability,
        ),
    ) -> Result<(), Error> {
        write_runtime_launch_snapshot_result(self, capability, builder)
    }
}

impl<'a, 'b>
    FromCapnp<(
        project_capnp::project_host::runtime_launch_snapshot_results::Reader<'a>,
        &'b Capability,
    )> for ProjectSideChannelResult
{
    fn from_capnp(
        (reader, capability): (
            project_capnp::project_host::runtime_launch_snapshot_results::Reader<'a>,
            &'b Capability,
        ),
    ) -> Result<Self, Error> {
        read_runtime_launch_snapshot_result_for_capability(reader, capability)
    }
}

impl<'a> ToCapnp<project_capnp::saved_document::Builder<'a>> for SavedDocument {
    fn to_capnp(&self, builder: project_capnp::saved_document::Builder<'a>) -> Result<(), Error> {
        write_saved_document(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::saved_document::Reader<'a>> for SavedDocument {
    fn from_capnp(reader: project_capnp::saved_document::Reader<'a>) -> Result<Self, Error> {
        read_saved_document(reader)
    }
}

impl<'a> ToCapnp<project_capnp::runtime_launch_snapshot_request::Builder<'a>>
    for RuntimeLaunchSnapshotRequest
{
    fn to_capnp(
        &self,
        builder: project_capnp::runtime_launch_snapshot_request::Builder<'a>,
    ) -> Result<(), Error> {
        write_runtime_launch_snapshot_request(self, builder)
    }
}

impl<'a> FromCapnp<project_capnp::runtime_launch_snapshot_request::Reader<'a>>
    for RuntimeLaunchSnapshotRequest
{
    fn from_capnp(
        reader: project_capnp::runtime_launch_snapshot_request::Reader<'a>,
    ) -> Result<Self, Error> {
        read_runtime_launch_snapshot_request(reader)
    }
}

fn read_uuid_data(bytes: &[u8]) -> Result<Uuid, Error> {
    Uuid::from_slice(bytes).map_err(|error| Error::failed(format!("invalid UUID bytes: {error}")))
}

fn read_project_document_request_fields(
    kind: &'static str,
    required_permission: &'static str,
    capability: core::core_capnp::capability::Reader<'_>,
    document: project_capnp::document_id::Reader<'_>,
) -> Result<ProjectDocumentRequest, Error> {
    let request = ProjectDocumentRequest {
        capability: core::Capability::from_capnp(capability)?,
        document_id: DocumentId::new(read_document_id(document)?),
    };
    validate_project_document_request_for_transport(kind, required_permission, &request)?;
    Ok(request)
}

fn validate_project_document_request_for_transport(
    kind: &'static str,
    required_permission: &'static str,
    request: &ProjectDocumentRequest,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        kind,
        &request.capability,
        required_permission,
    )?;
    validate_document_id_for_transport(kind, "document id", &request.document_id)
}

fn validate_create_graph_document_request_for_transport(
    request: &CreateGraphDocumentRequest,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "create graph document request",
        &request.capability,
        PROJECT_DOCUMENT_WRITE_PERMISSION,
    )?;
    validate_document_id_for_transport(
        "create graph document request",
        "document id",
        &request.document_id,
    )?;
    validate_non_empty_text(
        "create graph document request",
        "graph type",
        &request.graph_type,
    )
}

fn validate_node_source_link_request_for_transport(
    request: &NodeSourceLinkRequest,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "node source-link request",
        &request.capability,
        PROJECT_SOURCE_NAVIGATION_PERMISSION,
    )?;
    validate_node_source_link_for_transport("node source-link request", &request.source_link)
}

fn validate_node_source_link_target_for_transport(
    target: &NodeSourceLinkTarget,
) -> Result<(), Error> {
    validate_node_source_link_for_transport("node source-link target", &target.source_link)?;
    validate_optional_text(
        "node source-link target",
        "resolved path",
        target.resolved_path.as_deref(),
    )?;
    validate_optional_text(
        "node source-link target",
        "package id",
        target.package_id.as_deref(),
    )?;
    validate_optional_text(
        "node source-link target",
        "package root",
        target.package_root.as_deref(),
    )?;

    match target.path_kind {
        NodeSourceLinkPathKind::DocsOnly => {
            if target.source_link.docs_url.as_deref().is_none() {
                return Err(invalid_project_protocol_value(
                    "node source-link target",
                    "docs-only target must carry docs URL",
                ));
            }
        }
        NodeSourceLinkPathKind::Unresolved => {}
        NodeSourceLinkPathKind::Absolute
        | NodeSourceLinkPathKind::PackageRelative
        | NodeSourceLinkPathKind::WorkspaceRelative => {
            if target.resolved_path.as_deref().is_none() {
                return Err(invalid_project_protocol_value(
                    "node source-link target",
                    "resolved target must carry resolved path",
                ));
            }
        }
    }

    if target.path_kind == NodeSourceLinkPathKind::PackageRelative
        && (target.package_id.as_deref().is_none() || target.package_root.as_deref().is_none())
    {
        return Err(invalid_project_protocol_value(
            "node source-link target",
            "package-relative target must carry package id and package root",
        ));
    }

    Ok(())
}

fn validate_node_source_link_for_transport(
    kind: &'static str,
    link: &NodeSourceLink,
) -> Result<(), Error> {
    validate_optional_text(kind, "package", link.package.as_deref())?;
    validate_optional_text(kind, "module path", link.module_path.as_deref())?;
    validate_optional_text(kind, "symbol path", link.symbol_path.as_deref())?;
    validate_optional_text(kind, "file", link.file.as_deref())?;
    validate_optional_text(kind, "docs URL", link.docs_url.as_deref())?;

    if link.file.is_none()
        && link.docs_url.is_none()
        && link.symbol_path.is_none()
        && link.module_path.is_none()
    {
        return Err(invalid_project_protocol_value(
            kind,
            "source link must carry a file, docs URL, symbol path, or module path",
        ));
    }

    Ok(())
}

fn validate_project_host_capability_request_for_transport(
    kind: &'static str,
    capability: &Capability,
    required_permission: &'static str,
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_project_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.audience != PROJECT_HOST_AUDIENCE {
        return Err(invalid_project_protocol_value(
            kind,
            format!(
                "capability audience must be `{PROJECT_HOST_AUDIENCE}`, got `{}`",
                capability.audience
            ),
        ));
    }
    if !project_host_permission_allows_role(required_permission, capability.role) {
        return Err(invalid_project_protocol_value(
            kind,
            format!(
                "capability role {:?} is not allowed for `{required_permission}`",
                capability.role
            ),
        ));
    }
    if !capability.has_permissions(&[required_permission]) {
        return Err(invalid_project_protocol_value(
            kind,
            format!("capability missing `{required_permission}`"),
        ));
    }
    if capability.session.is_none() {
        return Err(invalid_project_protocol_value(
            kind,
            format!("`{required_permission}` capability must be session-scoped"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_project_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_project_inventory_report_for_transport(
    report: &ProjectInventoryReport,
) -> Result<(), Error> {
    if report.service_role.trim().is_empty() {
        return Err(invalid_project_protocol_value(
            "project inventory report",
            "service role cannot be empty",
        ));
    }
    for gem in &report.gems {
        if gem.id.trim().is_empty() {
            return Err(invalid_project_protocol_value(
                "project inventory report",
                "gem id cannot be empty",
            ));
        }
        if !gem.expected && !gem.active {
            return Err(invalid_project_protocol_value(
                "project inventory report",
                format!("gem `{}` is neither expected nor active", gem.id),
            ));
        }
        if gem.expected && gem.expected_package.trim().is_empty() {
            return Err(invalid_project_protocol_value(
                "project inventory report",
                format!("expected gem `{}` is missing its expected package", gem.id),
            ));
        }
    }
    Ok(())
}

fn project_host_permission_allows_role(permission: &str, role: core::ServiceRole) -> bool {
    match permission {
        PROJECT_SCHEMA_PERMISSION
        | PROJECT_GAMEDATA_PERMISSION
        | PROJECT_NODE_CATALOG_PERMISSION
        | PROJECT_GRAPH_CATALOG_PERMISSION
        | PROJECT_INVENTORY_PERMISSION => matches!(
            role,
            core::ServiceRole::Editor
                | core::ServiceRole::ProjectHost
                | core::ServiceRole::RuntimeHost
                | core::ServiceRole::SessionSupervisor
        ),
        PROJECT_EDIT_PERMISSION
        | PROJECT_DOCUMENT_READ_PERMISSION
        | PROJECT_RUNTIME_LAUNCH_PERMISSION
        | PROJECT_SOURCE_NAVIGATION_PERMISSION => {
            matches!(
                role,
                core::ServiceRole::Editor | core::ServiceRole::ProjectHost
            )
        }
        PROJECT_DOCUMENT_WRITE_PERMISSION => matches!(
            role,
            core::ServiceRole::Editor
                | core::ServiceRole::ProjectHost
                | core::ServiceRole::SessionSupervisor
        ),
        _ => false,
    }
}

fn validate_project_side_channel_result_for_transport(
    kind: &'static str,
    snapshot: &SideChannelHandle,
    required_permission: &'static str,
) -> Result<(), Error> {
    let Some(capability) = &snapshot.capability else {
        return Err(invalid_project_protocol_value(
            kind,
            "side-channel handle must carry the accepted request capability",
        ));
    };
    validate_project_host_capability_request_for_transport(kind, capability, required_permission)
}

fn validate_gamedata_catalog_snapshot_for_transport(
    catalog: &GameDataCatalogSnapshot,
) -> Result<(), Error> {
    if catalog.catalog_version != GAMEDATA_CATALOG_SNAPSHOT_VERSION {
        return Err(invalid_project_protocol_value(
            "GameData catalog snapshot",
            format!(
                "catalog version must be {GAMEDATA_CATALOG_SNAPSHOT_VERSION}, got {}",
                catalog.catalog_version
            ),
        ));
    }

    let mut table_names = BTreeSet::new();
    for table in &catalog.tables {
        validate_gamedata_table_descriptor(table)?;
        if !table_names.insert(table.name.as_str()) {
            return Err(invalid_project_protocol_value(
                "GameData catalog snapshot",
                format!("duplicate table descriptor `{}`", table.name),
            ));
        }
    }

    let mut family_names = BTreeSet::new();
    for family in &catalog.families {
        validate_non_empty_text("GameData table family descriptor", "name", &family.name)?;
        validate_non_empty_text(
            "GameData table family descriptor",
            "row type",
            &family.row_type,
        )?;
        validate_non_empty_text(
            "GameData table family descriptor",
            "duplicate key policy",
            &family.duplicate_key_policy,
        )?;
        if family.tables.is_empty() {
            return Err(invalid_project_protocol_value(
                "GameData table family descriptor",
                format!("family `{}` must list table members", family.name),
            ));
        }
        for table in &family.tables {
            validate_non_empty_text("GameData table family descriptor", "table", table)?;
        }
        if !family_names.insert(family.name.as_str()) {
            return Err(invalid_project_protocol_value(
                "GameData catalog snapshot",
                format!("duplicate table family descriptor `{}`", family.name),
            ));
        }
    }

    let mut manager_keys = BTreeSet::new();
    for manager in &catalog.managers {
        validate_gamedata_manager_entry(manager)?;
        if !manager_keys.insert(manager.key.as_str()) {
            return Err(invalid_project_protocol_value(
                "GameData catalog snapshot",
                format!("duplicate manager catalog key `{}`", manager.key),
            ));
        }
    }

    for diagnostic in &catalog.diagnostics {
        validate_gamedata_catalog_diagnostic(diagnostic)?;
    }

    Ok(())
}

/// Validates one `GameData` table descriptor in isolation.
///
/// Cross-table uniqueness stays with the caller, which owns the seen-name set.
fn validate_gamedata_table_descriptor(table: &GameDataTableDescriptor) -> Result<(), Error> {
    validate_non_empty_text("GameData table descriptor", "name", &table.name)?;
    validate_non_empty_text("GameData table descriptor", "row type", &table.row_type)?;
    validate_non_empty_text(
        "GameData table descriptor",
        "source root",
        &table.source_root,
    )?;
    validate_non_empty_text(
        "GameData table descriptor",
        "source path",
        &table.source_path,
    )?;
    validate_non_empty_text(
        "GameData table descriptor",
        "document id",
        &table.document_id,
    )?;
    validate_project_relative_path(
        "GameData table descriptor",
        "document id",
        &table.document_id,
    )?;
    validate_non_empty_text(
        "GameData table descriptor",
        "schema type",
        &table.schema_type,
    )?;
    validate_non_empty_text(
        "GameData table descriptor",
        "source root key",
        &table.source_ref.source_root_key,
    )?;
    validate_non_empty_text(
        "GameData table descriptor",
        "source schema type",
        &table.source_ref.schema_type,
    )?;
    if table.source_ref.source_path != table.source_path {
        return Err(invalid_project_protocol_value(
            "GameData table descriptor",
            format!(
                "source ref path `{}` does not match source path `{}`",
                table.source_ref.source_path, table.source_path
            ),
        ));
    }
    validate_non_empty_text("GameData table descriptor", "category", &table.category)?;
    for family in &table.families {
        validate_non_empty_text("GameData table descriptor", "family", family)?;
    }
    Ok(())
}

/// Validates one `GameData` manager catalog entry in isolation.
///
/// Cross-entry key uniqueness stays with the caller, which owns the seen-key set.
fn validate_gamedata_manager_entry(manager: &GameDataManagerCatalogEntry) -> Result<(), Error> {
    validate_non_empty_text("GameData manager catalog entry", "key", &manager.key)?;
    validate_non_empty_text("GameData manager catalog entry", "name", &manager.name)?;
    validate_non_empty_text(
        "GameData manager catalog entry",
        "row type",
        &manager.row_type,
    )?;
    validate_non_empty_text("GameData manager catalog entry", "kind", &manager.kind)?;
    validate_non_empty_text(
        "GameData manager catalog entry",
        "output type",
        &manager.output_type,
    )?;
    validate_non_empty_text(
        "GameData manager catalog entry",
        "key policy kind",
        &manager.key_policy.kind,
    )?;
    validate_non_empty_text(
        "GameData manager catalog entry",
        "duplicate key policy",
        &manager.duplicate_key_policy,
    )?;
    if manager.projection_hash.len() != blake3::OUT_LEN {
        return Err(invalid_project_protocol_value(
            "GameData manager catalog entry",
            format!(
                "projection hash for `{}` must have {} bytes, got {}",
                manager.key,
                blake3::OUT_LEN,
                manager.projection_hash.len()
            ),
        ));
    }
    if let Some(target) = &manager.provider_target {
        validate_gamedata_provider_target(target)?;
    }
    for target in &manager.source_targets {
        validate_gamedata_provider_target(target)?;
    }
    for input in &manager.inputs {
        validate_non_empty_text("GameData manager input", "kind", &input.kind)?;
        validate_non_empty_text("GameData manager input", "name", &input.name)?;
    }
    for transform in &manager.projection_transforms {
        validate_non_empty_text("GameData projection transform", "field", &transform.field)?;
        validate_non_empty_text("GameData projection transform", "kind", &transform.kind)?;
    }
    for dependency in manager.dependencies.iter().chain(&manager.dependents) {
        validate_gamedata_manager_node_ref(dependency)?;
    }
    for diagnostic in &manager.diagnostics {
        validate_gamedata_catalog_diagnostic(diagnostic)?;
    }
    Ok(())
}

fn validate_gamedata_provider_target(target: &GameDataProviderTarget) -> Result<(), Error> {
    validate_non_empty_text("GameData provider target", "kind", &target.kind)?;
    validate_non_empty_text("GameData provider target", "name", &target.name)?;
    validate_non_empty_text("GameData provider target", "row type", &target.row_type)
}

fn validate_gamedata_manager_node_ref(node: &GameDataManagerNodeRef) -> Result<(), Error> {
    validate_non_empty_text("GameData manager node ref", "key", &node.key)?;
    validate_non_empty_text("GameData manager node ref", "label", &node.label)?;
    validate_non_empty_text("GameData manager node ref", "kind", &node.kind)
}

fn validate_gamedata_catalog_diagnostic(
    diagnostic: &GameDataCatalogDiagnostic,
) -> Result<(), Error> {
    validate_non_empty_text("GameData catalog diagnostic", "code", &diagnostic.code)?;
    validate_non_empty_text(
        "GameData catalog diagnostic",
        "message",
        &diagnostic.message,
    )
}

fn validate_runtime_launch_side_channel_result_for_transport(
    kind: &'static str,
    snapshot: &SideChannelHandle,
) -> Result<(), Error> {
    let Some(capability) = &snapshot.capability else {
        return Err(invalid_project_protocol_value(
            kind,
            "side-channel handle must carry the accepted runtime launch capability",
        ));
    };
    validate_project_runtime_launch_side_channel_capability(kind, capability)
}

fn validate_project_runtime_launch_side_channel_capability(
    kind: &'static str,
    capability: &Capability,
) -> Result<(), Error> {
    if capability.is_empty() {
        return Err(invalid_project_protocol_value(
            kind,
            "capability cannot be empty",
        ));
    }
    if capability.service.namespace != PROJECT_HOST_NAMESPACE
        || capability.service.name != PROJECT_HOST_SERVICE_NAME
    {
        return Err(invalid_project_protocol_value(
            kind,
            format!(
                "runtime launch capability must be issued to `{PROJECT_HOST_NAMESPACE}/{PROJECT_HOST_SERVICE_NAME}`, got `{}/{}`",
                capability.service.namespace, capability.service.name
            ),
        ));
    }
    if capability.role != core::ServiceRole::ProjectHost {
        return Err(invalid_project_protocol_value(
            kind,
            format!(
                "runtime launch capability role must be {:?}, got {:?}",
                core::ServiceRole::ProjectHost,
                capability.role
            ),
        ));
    }
    if capability.audience != RUNTIME_HOST_AUDIENCE {
        return Err(invalid_project_protocol_value(
            kind,
            format!(
                "capability audience must be `{RUNTIME_HOST_AUDIENCE}`, got `{}`",
                capability.audience
            ),
        ));
    }
    if !capability.has_permissions(&[RUNTIME_CONTROL_PERMISSION]) {
        return Err(invalid_project_protocol_value(
            kind,
            format!("capability missing `{RUNTIME_CONTROL_PERMISSION}`"),
        ));
    }
    if capability.session.is_none() {
        return Err(invalid_project_protocol_value(
            kind,
            format!("`{RUNTIME_CONTROL_PERMISSION}` capability must be session-scoped"),
        ));
    }
    capability
        .validate_lifetime()
        .map_err(|error| invalid_project_protocol_value(kind, error.to_string()))?;
    Ok(())
}

fn validate_project_side_channel_result_matches_capability(
    kind: &'static str,
    snapshot: &SideChannelHandle,
    expected: &Capability,
) -> Result<(), Error> {
    core::validate_side_channel_capability_matches(snapshot, expected, kind)
        .map_err(|error| invalid_project_protocol_value(kind, error.to_string()))
}

fn validate_graph_command_batch_request_for_transport(
    kind: &'static str,
    request: &GraphCommandBatchRequest,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        kind,
        &request.capability,
        PROJECT_EDIT_PERMISSION,
    )?;
    validate_project_side_channel_result_for_transport(
        kind,
        &request.batch,
        PROJECT_EDIT_PERMISSION,
    )?;
    validate_project_side_channel_result_matches_capability(
        kind,
        &request.batch,
        &request.capability,
    )
}

fn validate_graph_command_batch_snapshot_for_transport(
    batch: &GraphCommandBatchSnapshot,
) -> Result<(), Error> {
    validate_document_id_for_transport(
        "graph command batch snapshot",
        "document id",
        &batch.document_id,
    )?;
    validate_non_empty_text(
        "graph command batch snapshot",
        "client batch id",
        &batch.client_batch_id,
    )?;
    validate_non_zero_count(
        "graph command batch snapshot",
        "command count",
        batch.commands.len(),
    )?;
    for command in &batch.commands {
        validate_graph_command_for_transport(command)?;
    }
    Ok(())
}

fn validate_graph_command_status_snapshot_for_transport(
    status: &GraphCommandStatusSnapshot,
) -> Result<(), Error> {
    validate_document_id_for_transport(
        "graph command status snapshot",
        "document id",
        &status.document_id,
    )?;
    validate_non_empty_text(
        "graph command status snapshot",
        "client batch id",
        &status.client_batch_id,
    )?;
    match &status.outcome {
        GraphCommandStatusOutcome::Accepted { revision } if revision.0 == 0 => {
            return Err(invalid_project_protocol_value(
                "graph command status snapshot",
                "accepted revision must be greater than zero",
            ));
        }
        GraphCommandStatusOutcome::Accepted { .. } => {}
        GraphCommandStatusOutcome::Rejected { reason, .. } => {
            validate_non_empty_text("graph command status snapshot", "rejection reason", reason)?;
        }
    }
    for diagnostic in &status.diagnostics {
        validate_graph_command_diagnostic_for_transport(diagnostic)?;
    }
    Ok(())
}

fn validate_graph_document_snapshot_for_transport(
    snapshot: &GraphDocumentSnapshot,
) -> Result<(), Error> {
    validate_document_id_for_transport(
        "graph document snapshot",
        "document id",
        &snapshot.document_id,
    )?;
    validate_visual_graph_document_for_transport(&snapshot.document)
}

fn validate_visual_graph_document_for_transport(
    document: &VisualGraphDocument,
) -> Result<(), Error> {
    if document.document_version == 0 {
        return Err(invalid_project_protocol_value(
            "visual graph document",
            "document version 0 is reserved",
        ));
    }
    validate_non_empty_text("visual graph document", "graph type", &document.graph_type)?;
    if let Some(hash) = &document.required_catalog_hash
        && hash.len() != blake3::OUT_LEN
    {
        return Err(invalid_project_protocol_value(
            "visual graph document",
            format!(
                "required catalog hash must have {} bytes, got {}",
                blake3::OUT_LEN,
                hash.len()
            ),
        ));
    }

    let mut node_ids = BTreeSet::new();
    for node in &document.nodes {
        if !node_ids.insert(node.id) {
            return Err(invalid_project_protocol_value(
                "visual graph document",
                format!("duplicate graph node id {}", node.id),
            ));
        }
        validate_graph_node_for_transport(node)?;
    }

    let mut connection_ids = BTreeSet::new();
    for connection in &document.connections {
        if !connection_ids.insert(connection.id) {
            return Err(invalid_project_protocol_value(
                "visual graph document",
                format!("duplicate graph connection id {}", connection.id),
            ));
        }
        validate_graph_connection_for_transport(connection)?;
    }

    let mut comment_ids = BTreeSet::new();
    for comment in &document.comments {
        if !comment_ids.insert(comment.id) {
            return Err(invalid_project_protocol_value(
                "visual graph document",
                format!("duplicate graph comment id {}", comment.id),
            ));
        }
        validate_graph_comment_for_transport(comment)?;
    }

    Ok(())
}

fn validate_graph_command_for_transport(command: &GraphCommand) -> Result<(), Error> {
    match command {
        GraphCommand::AddNode { node } => validate_graph_node_for_transport(node),
        // Removal and disconnect commands carry only ids, which the enum
        // already constrains; there is nothing further to validate.
        GraphCommand::RemoveNode { .. }
        | GraphCommand::Disconnect { .. }
        | GraphCommand::RemoveComment { .. } => Ok(()),
        GraphCommand::SetInputValue { port_id, value, .. } => {
            validate_graph_port_id_for_transport("graph set input value command", *port_id)?;
            if let Some(value) = value {
                validate_reflected_graph_value_for_transport(value)?;
            }
            Ok(())
        }
        GraphCommand::MoveNode { layout, .. } => {
            validate_graph_node_layout_for_transport("graph move node command", *layout)
        }
        GraphCommand::Connect { connection } => validate_graph_connection_for_transport(connection),
        GraphCommand::SetConnectionRoute { route, .. } => {
            validate_graph_connection_route_for_transport(
                "graph set connection route command",
                route,
            )
        }
        GraphCommand::UpsertComment { comment } => validate_graph_comment_for_transport(comment),
    }
}

fn validate_graph_node_for_transport(node: &GraphNode) -> Result<(), Error> {
    validate_non_empty_text("graph node", "node type", node.node_type.as_str())?;
    if node.node_type_version == 0 {
        return Err(invalid_project_protocol_value(
            "graph node",
            "node type version 0 is reserved",
        ));
    }
    validate_graph_node_layout_for_transport("graph node", node.layout)?;
    for (port_id, value) in &node.input_values {
        validate_graph_port_id_for_transport("graph node input value", *port_id)?;
        validate_reflected_graph_value_for_transport(value)?;
    }
    Ok(())
}

fn validate_reflected_graph_value_for_transport(
    value: &vnext::ReflectedValueEnvelope,
) -> Result<(), Error> {
    validate_non_empty_text("reflected graph value", "type path", &value.type_path)?;
    if value.encoding == vnext::ReflectedValueEncoding::TypedRon {
        let payload = std::str::from_utf8(&value.payload).map_err(|error| {
            invalid_project_protocol_value(
                "reflected graph value",
                format!("typed RON payload is not UTF-8: {error}"),
            )
        })?;
        validate_non_empty_text("reflected graph value", "typed RON payload", payload)?;
    }
    Ok(())
}

fn validate_graph_connection_for_transport(connection: &GraphConnection) -> Result<(), Error> {
    validate_graph_port_ref_for_transport("graph connection from", &connection.from)?;
    validate_graph_port_ref_for_transport("graph connection to", &connection.to)?;
    validate_graph_connection_route_for_transport("graph connection route", &connection.route)
}

fn validate_graph_port_ref_for_transport(
    kind: &'static str,
    port: &GraphPortRef,
) -> Result<(), Error> {
    validate_graph_port_id_for_transport(kind, port.port_id)
}

fn validate_graph_connection_route_for_transport(
    kind: &'static str,
    route: &GraphConnectionRoute,
) -> Result<(), Error> {
    let mut seen_anchor_ids = BTreeSet::new();
    for anchor in &route.anchors {
        if !seen_anchor_ids.insert(anchor.id) {
            return Err(invalid_project_protocol_value(
                kind,
                format!("duplicate route anchor id {}", anchor.id),
            ));
        }
        validate_graph_point_for_transport(kind, "route anchor", anchor.position)?;
    }
    Ok(())
}

fn validate_graph_comment_for_transport(comment: &GraphComment) -> Result<(), Error> {
    validate_graph_comment_bounds_for_transport("graph comment", comment.bounds)
}

fn validate_graph_command_diagnostic_for_transport(
    diagnostic: &GraphCommandDiagnostic,
) -> Result<(), Error> {
    validate_non_empty_text("graph command diagnostic", "message", &diagnostic.message)
}

fn validate_graph_port_id_for_transport(
    kind: &'static str,
    port_id: NodePortId,
) -> Result<(), Error> {
    if port_id.is_reserved() {
        return Err(invalid_project_protocol_value(
            kind,
            "port id 0 is reserved",
        ));
    }
    Ok(())
}

fn validate_graph_node_layout_for_transport(
    kind: &'static str,
    layout: GraphNodeLayout,
) -> Result<(), Error> {
    validate_finite_f32(kind, "layout x", layout.x)?;
    validate_finite_f32(kind, "layout y", layout.y)
}

fn validate_graph_point_for_transport(
    kind: &'static str,
    label: &'static str,
    point: GraphPoint,
) -> Result<(), Error> {
    validate_finite_f32(kind, &format!("{label} x"), point.x)?;
    validate_finite_f32(kind, &format!("{label} y"), point.y)
}

fn validate_graph_comment_bounds_for_transport(
    kind: &'static str,
    bounds: GraphCommentBounds,
) -> Result<(), Error> {
    validate_finite_f32(kind, "bounds x", bounds.x)?;
    validate_finite_f32(kind, "bounds y", bounds.y)?;
    validate_finite_f32(kind, "bounds width", bounds.width)?;
    validate_finite_f32(kind, "bounds height", bounds.height)
}

fn validate_saved_document_for_transport(saved: &SavedDocument) -> Result<(), Error> {
    validate_document_id_for_transport("saved document", "document id", &saved.document_id)?;
    validate_project_relative_path("saved document", "source path", &saved.source_path)?;
    if saved.source_path != saved.document_id.as_str() {
        return Err(invalid_project_protocol_value(
            "saved document",
            format!(
                "source path `{}` does not match document id `{}`",
                saved.source_path,
                saved.document_id.as_str()
            ),
        ));
    }
    validate_non_empty_text("saved document", "schema type", &saved.schema_type)?;
    if saved.content_hash.len() != 32 {
        return Err(invalid_project_protocol_value(
            "saved document",
            format!(
                "content hash must be 32 bytes, got {}",
                saved.content_hash.len()
            ),
        ));
    }
    Ok(())
}

fn validate_save_document_result_for_transport(result: &SaveDocumentResult) -> Result<(), Error> {
    validate_saved_document_for_transport(&result.saved)?;
    if result.revision != result.saved.revision {
        return Err(invalid_project_protocol_value(
            "save document result",
            format!(
                "revision {} does not match saved document revision {}",
                result.revision.0, result.saved.revision.0
            ),
        ));
    }
    Ok(())
}

fn validate_runtime_launch_snapshot_request_for_transport(
    request: &RuntimeLaunchSnapshotRequest,
) -> Result<(), Error> {
    validate_project_host_capability_request_for_transport(
        "runtime launch snapshot request",
        &request.capability,
        PROJECT_RUNTIME_LAUNCH_PERMISSION,
    )?;
    validate_project_runtime_launch_side_channel_capability(
        "runtime launch snapshot request",
        &request.runtime_launch_capability,
    )?;
    validate_non_empty_text(
        "runtime launch snapshot request",
        "project id",
        &request.project_id,
    )?;
    if request.session_id == Uuid::nil() {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            "session id cannot be nil",
        ));
    }
    if request.capability.session != Some(request.session_id) {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!(
                "project-host capability session must be `{}`, got `{}`",
                request.session_id,
                request
                    .capability
                    .session
                    .map_or_else(|| "<none>".to_string(), |session| session.to_string())
            ),
        ));
    }
    if request.runtime_launch_capability.session != Some(request.session_id) {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!(
                "runtime launch capability session must be `{}`, got `{}`",
                request.session_id,
                request
                    .runtime_launch_capability
                    .session
                    .map_or_else(|| "<none>".to_string(), |session| session.to_string())
            ),
        ));
    }
    validate_non_empty_text(
        "runtime launch snapshot request",
        "session slug",
        &request.session_slug,
    )?;
    validate_non_empty_text(
        "runtime launch snapshot request",
        "project root",
        &request.project_root,
    )?;
    validate_non_empty_text(
        "runtime launch snapshot request",
        "workspace path",
        &request.workspace_path,
    )?;
    validate_positive_i64(
        "runtime launch snapshot request",
        "workspace id",
        request.workspace_id,
    )?;
    validate_non_empty_text(
        "runtime launch snapshot request",
        "launch profile",
        &request.launch_profile,
    )?;
    if request.asset_source_roots.is_empty() {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            "asset source roots cannot be empty",
        ));
    }

    validate_runtime_launch_source_roots(request)?;

    validate_runtime_launch_project_assets_root(request)?;
    validate_runtime_launch_package_roots_for_transport(&request.asset_package_roots)?;

    Ok(())
}

/// Checks every declared asset source root against the request's workspace and
/// rejects duplicate root ids or portable keys.
fn validate_runtime_launch_source_roots(
    request: &RuntimeLaunchSnapshotRequest,
) -> Result<(), Error> {
    let mut seen_source_root_ids = BTreeSet::new();
    let mut seen_portable_keys = BTreeSet::new();
    for root in &request.asset_source_roots {
        if root.workspace_id != request.workspace_id {
            return Err(invalid_project_protocol_value(
                "runtime launch snapshot request",
                format!(
                    "asset source root `{}` belongs to workspace {}, expected {}",
                    root.portable_key, root.workspace_id, request.workspace_id
                ),
            ));
        }
        validate_positive_i64(
            "runtime launch snapshot request",
            "workspace root id",
            root.workspace_root_id,
        )?;
        validate_positive_i64("runtime launch snapshot request", "root id", root.root_id)?;
        validate_non_empty_text(
            "runtime launch snapshot request",
            "source root owner id",
            &root.owner_id,
        )?;
        validate_non_empty_text(
            "runtime launch snapshot request",
            "source root path",
            &root.source_root,
        )?;
        validate_non_empty_text(
            "runtime launch snapshot request",
            "source root portable key",
            &root.portable_key,
        )?;
        if !seen_source_root_ids.insert(root.workspace_root_id) {
            return Err(invalid_project_protocol_value(
                "runtime launch snapshot request",
                format!("duplicate source root id {}", root.workspace_root_id),
            ));
        }
        if !seen_portable_keys.insert(root.portable_key.as_str()) {
            return Err(invalid_project_protocol_value(
                "runtime launch snapshot request",
                format!("duplicate source root key `{}`", root.portable_key),
            ));
        }
    }
    Ok(())
}

/// Checks that the request carries the DB-owned `project:<id>:assets` root and
/// that it is a workspace-internal, prefix-free root owned by the project.
fn validate_runtime_launch_project_assets_root(
    request: &RuntimeLaunchSnapshotRequest,
) -> Result<(), Error> {
    let project_assets_key = format!("project:{}:assets", request.project_id);
    let Some(project_assets) = request
        .asset_source_roots
        .iter()
        .find(|root| root.portable_key == project_assets_key)
    else {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!("missing DB-owned project assets root `{project_assets_key}`"),
        ));
    };
    if project_assets.owner_id != request.project_id {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!(
                "project assets root `{project_assets_key}` owner `{}` does not match project `{}`",
                project_assets.owner_id, request.project_id
            ),
        ));
    }
    let project_assets_path = Path::new(&project_assets.source_root);
    let workspace_path = Path::new(&request.workspace_path);
    if project_assets_path == workspace_path || !project_assets_path.starts_with(workspace_path) {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!(
                "project assets root `{project_assets_key}` path `{}` must be inside workspace `{}`",
                project_assets.source_root, request.workspace_path
            ),
        ));
    }
    if !project_assets.is_root {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!("project assets root `{project_assets_key}` must be marked as a root source"),
        ));
    }
    if !project_assets.output_prefix.is_empty() {
        return Err(invalid_project_protocol_value(
            "runtime launch snapshot request",
            format!("project assets root `{project_assets_key}` must use an empty output prefix"),
        ));
    }
    Ok(())
}

fn validate_runtime_launch_package_roots_for_transport(
    package_roots: &[RuntimeAssetPackageRoot],
) -> Result<(), Error> {
    validate_runtime_asset_package_roots(package_roots).map_err(|error| {
        invalid_project_protocol_value("runtime launch snapshot request", error.to_string())
    })
}

fn validate_document_id_for_transport(
    kind: &str,
    label: &str,
    document_id: &DocumentId,
) -> Result<(), Error> {
    validate_project_relative_path(kind, label, document_id.as_str())
}

fn validate_project_relative_path(kind: &str, label: &str, value: &str) -> Result<(), Error> {
    if !is_safe_project_relative_path(value) {
        return Err(invalid_project_protocol_value(
            kind,
            format!("{label} `{value}` must be a project-relative source path"),
        ));
    }
    Ok(())
}

fn is_safe_project_relative_path(value: &str) -> bool {
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.contains('\\')
        || value.contains(':')
        || value.trim() != value
    {
        return false;
    }

    let mut has_component = false;
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return false;
        }
        has_component = true;
    }
    has_component
}

fn validate_positive_i64(kind: &str, label: &str, value: i64) -> Result<(), Error> {
    if value <= 0 {
        return Err(invalid_project_protocol_value(
            kind,
            format!("{label} must be positive, got {value}"),
        ));
    }
    Ok(())
}

fn validate_non_empty_text(kind: &str, label: &str, value: &str) -> Result<(), Error> {
    if value.trim().is_empty() {
        return Err(invalid_project_protocol_value(
            kind,
            format!("{label} cannot be empty"),
        ));
    }
    Ok(())
}

fn validate_optional_text(kind: &str, label: &str, value: Option<&str>) -> Result<(), Error> {
    if let Some(value) = value {
        validate_non_empty_text(kind, label, value)?;
    }
    Ok(())
}

fn validate_non_zero_count(kind: &str, label: &str, value: usize) -> Result<(), Error> {
    if value == 0 {
        return Err(invalid_project_protocol_value(
            kind,
            format!("{label} must be greater than zero"),
        ));
    }
    Ok(())
}

fn validate_finite_f32(kind: &str, label: &str, value: f32) -> Result<(), Error> {
    if !value.is_finite() {
        return Err(invalid_project_protocol_value(
            kind,
            format!("{label} must be finite"),
        ));
    }
    Ok(())
}

/// Narrows a `usize` list length or element index to the `u32` Cap'n Proto
/// uses for both.
///
/// The casts this replaces were lossy on 64-bit targets: an oversized
/// collection would have silently written a wrapped length and produced a
/// message that decodes to the wrong number of elements. Failing the write is
/// the honest outcome, and every caller is already in a `Result`.
///
/// # Errors
///
/// Returns an error if `value` does not fit in a `u32`.
pub(crate) fn capnp_list_index(value: usize) -> Result<u32, Error> {
    u32::try_from(value)
        .map_err(|_| Error::failed("Cap'n Proto list length exceeds u32 range".to_string()))
}

/// Narrows a loop index that is already bounded by a Cap'n Proto list length.
///
/// Callers iterate a slice whose length was passed to `init_*` through
/// [`capnp_list_index`], so the list only exists if that length fit in a `u32`
/// — every index below it fits too. This is the in-range counterpart to
/// [`capnp_list_index`], for writers that cannot report a failure.
pub(crate) fn capnp_bounded_index(index: usize) -> u32 {
    u32::try_from(index).unwrap_or_else(|_| {
        unreachable!("list index {index} exceeds the length already narrowed to u32")
    })
}

fn invalid_project_protocol_value(kind: &str, reason: impl Into<String>) -> Error {
    Error::failed(format!("invalid {kind}: {}", reason.into()))
}

/// Serializes a `GameData` catalog snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the `GameData` catalog snapshot fails its transport validation, or if
/// the message runs out of space while writing it.
pub fn encode_gamedata_catalog_snapshot(
    catalog: &GameDataCatalogSnapshot,
) -> Result<Vec<u8>, Error> {
    validate_gamedata_catalog_snapshot_for_transport(catalog)?;

    let mut message = message::Builder::new_default();
    let root = message.init_root::<project_capnp::game_data_catalog_snapshot::Builder<'_>>();
    write_gamedata_catalog_snapshot(catalog, root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a `GameData` catalog snapshot from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or if a field of
/// the `GameData` catalog snapshot is absent from the message or is not valid UTF-8.
pub fn decode_gamedata_catalog_snapshot_packed(
    bytes: &[u8],
) -> Result<GameDataCatalogSnapshot, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<project_capnp::game_data_catalog_snapshot::Reader<'_>>()?;
    read_gamedata_catalog_snapshot(root)
}

/// # Errors
///
/// Returns [`GameDataCatalogSideChannelError::InvalidHandle`] if the staging file behind
/// `handle` cannot be read or fails its hash verification, and
/// [`GameDataCatalogSideChannelError::Decode`] for any error
/// [`decode_gamedata_catalog_snapshot_packed`] returns for those bytes.
pub fn load_gamedata_catalog_side_channel(
    handle: &SideChannelHandle,
) -> Result<GameDataCatalogSnapshot, GameDataCatalogSideChannelError> {
    let file = az_proto_core::read_verified_staging_file(handle)?;
    Ok(decode_gamedata_catalog_snapshot_packed(&file.bytes)?)
}

#[derive(Debug, Error)]
pub enum GameDataCatalogSideChannelError {
    #[error("invalid GameData catalog side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode GameData catalog snapshot: {0}")]
    Decode(#[from] Error),
}

/// Serializes a node type catalog snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the node type catalog snapshot fails its transport validation, or if the
/// message runs out of space while writing it.
pub fn encode_node_type_catalog_snapshot(catalog: &NodeTypeCatalog) -> Result<Vec<u8>, Error> {
    catalog
        .validate()
        .map_err(|error| Error::failed(format!("invalid node type catalog snapshot: {error}")))?;

    let mut message = message::Builder::new_default();
    let root = message.init_root::<project_capnp::node_type_catalog_snapshot::Builder<'_>>();
    write_node_type_catalog_snapshot(catalog, root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a node type catalog snapshot from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or if a field of
/// the node type catalog snapshot is absent from the message or is not valid UTF-8.
pub fn decode_node_type_catalog_snapshot_packed(bytes: &[u8]) -> Result<NodeTypeCatalog, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<project_capnp::node_type_catalog_snapshot::Reader<'_>>()?;
    read_node_type_catalog_snapshot(root)
}

/// # Errors
///
/// Returns [`NodeTypeCatalogSideChannelError::InvalidHandle`] if the staging file behind
/// `handle` cannot be read or fails its hash verification, and
/// [`NodeTypeCatalogSideChannelError::Decode`] for any error
/// [`decode_node_type_catalog_snapshot_packed`] returns for those bytes.
pub fn load_node_type_catalog_side_channel(
    handle: &SideChannelHandle,
) -> Result<NodeTypeCatalog, NodeTypeCatalogSideChannelError> {
    let file = az_proto_core::read_verified_staging_file(handle)?;
    Ok(decode_node_type_catalog_snapshot_packed(&file.bytes)?)
}

#[derive(Debug, Error)]
pub enum NodeTypeCatalogSideChannelError {
    #[error("invalid node type catalog side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode node type catalog snapshot: {0}")]
    Decode(#[from] Error),
}

/// Serializes a graph type catalog snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the graph type catalog snapshot fails its transport validation, or if
/// the message runs out of space while writing it.
pub fn encode_graph_type_catalog_snapshot(catalog: &GraphTypeCatalog) -> Result<Vec<u8>, Error> {
    catalog
        .validate()
        .map_err(|error| Error::failed(format!("invalid graph type catalog snapshot: {error}")))?;

    let mut message = message::Builder::new_default();
    let root = message.init_root::<project_capnp::graph_type_catalog_snapshot::Builder<'_>>();
    write_graph_type_catalog_snapshot(catalog, root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a graph type catalog snapshot from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or if a field of
/// the graph type catalog snapshot is absent from the message or is not valid UTF-8.
pub fn decode_graph_type_catalog_snapshot_packed(bytes: &[u8]) -> Result<GraphTypeCatalog, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<project_capnp::graph_type_catalog_snapshot::Reader<'_>>()?;
    read_graph_type_catalog_snapshot(root)
}

/// # Errors
///
/// Returns [`GraphTypeCatalogSideChannelError::InvalidHandle`] if the staging file behind
/// `handle` cannot be read or fails its hash verification, and
/// [`GraphTypeCatalogSideChannelError::Decode`] for any error
/// [`decode_graph_type_catalog_snapshot_packed`] returns for those bytes.
pub fn load_graph_type_catalog_side_channel(
    handle: &SideChannelHandle,
) -> Result<GraphTypeCatalog, GraphTypeCatalogSideChannelError> {
    let file = az_proto_core::read_verified_staging_file(handle)?;
    Ok(decode_graph_type_catalog_snapshot_packed(&file.bytes)?)
}

#[derive(Debug, Error)]
pub enum GraphTypeCatalogSideChannelError {
    #[error("invalid graph type catalog side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode graph type catalog snapshot: {0}")]
    Decode(#[from] Error),
}

/// Serializes a graph command batch snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the graph command batch snapshot fails its transport validation, or if
/// the message runs out of space while writing it.
pub fn encode_graph_command_batch_snapshot(
    batch: &GraphCommandBatchSnapshot,
) -> Result<Vec<u8>, Error> {
    validate_graph_command_batch_snapshot_for_transport(batch)?;

    let mut message = message::Builder::new_default();
    let root = message.init_root::<project_capnp::graph_command_batch_snapshot::Builder<'_>>();
    write_graph_command_batch_snapshot(batch, root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a graph command batch snapshot from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or if a field of
/// the graph command batch snapshot is absent from the message or is not valid UTF-8.
pub fn decode_graph_command_batch_snapshot_packed(
    bytes: &[u8],
) -> Result<GraphCommandBatchSnapshot, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<project_capnp::graph_command_batch_snapshot::Reader<'_>>()?;
    read_graph_command_batch_snapshot(root)
}

/// # Errors
///
/// Returns [`GraphCommandBatchSideChannelError::InvalidHandle`] if the staging file behind
/// `handle` cannot be read or fails its hash verification, and
/// [`GraphCommandBatchSideChannelError::Decode`] for any error
/// [`decode_graph_command_batch_snapshot_packed`] returns for those bytes.
pub fn load_graph_command_batch_side_channel(
    handle: &SideChannelHandle,
) -> Result<GraphCommandBatchSnapshot, GraphCommandBatchSideChannelError> {
    let file = az_proto_core::read_verified_staging_file(handle)?;
    Ok(decode_graph_command_batch_snapshot_packed(&file.bytes)?)
}

#[derive(Debug, Error)]
pub enum GraphCommandBatchSideChannelError {
    #[error("invalid graph command batch side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode graph command batch snapshot: {0}")]
    Decode(#[from] Error),
}

/// Serializes a graph command status snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the graph command status snapshot fails its transport validation, or if
/// the message runs out of space while writing it.
pub fn encode_graph_command_status_snapshot(
    status: &GraphCommandStatusSnapshot,
) -> Result<Vec<u8>, Error> {
    validate_graph_command_status_snapshot_for_transport(status)?;

    let mut message = message::Builder::new_default();
    let root = message.init_root::<project_capnp::graph_command_status_snapshot::Builder<'_>>();
    write_graph_command_status_snapshot(status, root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a graph command status snapshot from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or if a field of
/// the graph command status snapshot is absent from the message or is not valid UTF-8.
pub fn decode_graph_command_status_snapshot_packed(
    bytes: &[u8],
) -> Result<GraphCommandStatusSnapshot, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<project_capnp::graph_command_status_snapshot::Reader<'_>>()?;
    read_graph_command_status_snapshot(root)
}

/// # Errors
///
/// Returns [`GraphCommandStatusSideChannelError::InvalidHandle`] if the staging file behind
/// `handle` cannot be read or fails its hash verification, and
/// [`GraphCommandStatusSideChannelError::Decode`] for any error
/// [`decode_graph_command_status_snapshot_packed`] returns for those bytes.
pub fn load_graph_command_status_side_channel(
    handle: &SideChannelHandle,
) -> Result<GraphCommandStatusSnapshot, GraphCommandStatusSideChannelError> {
    let file = az_proto_core::read_verified_staging_file(handle)?;
    Ok(decode_graph_command_status_snapshot_packed(&file.bytes)?)
}

#[derive(Debug, Error)]
pub enum GraphCommandStatusSideChannelError {
    #[error("invalid graph command status side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode graph command status snapshot: {0}")]
    Decode(#[from] Error),
}

/// Serializes a graph document snapshot for a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if the graph document snapshot fails its transport validation, or if the
/// message runs out of space while writing it.
pub fn encode_graph_document_snapshot(snapshot: &GraphDocumentSnapshot) -> Result<Vec<u8>, Error> {
    validate_graph_document_snapshot_for_transport(snapshot)?;

    let mut message = message::Builder::new_default();
    let root = message.init_root::<project_capnp::graph_document_snapshot::Builder<'_>>();
    write_graph_document_snapshot(snapshot, root)?;

    let mut bytes = Vec::new();
    serialize_packed::write_message(&mut bytes, &message)?;
    Ok(bytes)
}

/// Deserializes a graph document snapshot from a side-channel blob payload.
///
/// # Errors
///
/// Returns an error if `bytes` is not a readable packed Cap'n Proto message, or if a field of
/// the graph document snapshot is absent from the message or is not valid UTF-8.
pub fn decode_graph_document_snapshot_packed(bytes: &[u8]) -> Result<GraphDocumentSnapshot, Error> {
    let reader =
        serialize_packed::read_message(&mut Cursor::new(bytes), message::ReaderOptions::new())?;
    let root = reader.get_root::<project_capnp::graph_document_snapshot::Reader<'_>>()?;
    read_graph_document_snapshot(root)
}

/// # Errors
///
/// Returns [`GraphDocumentSideChannelError::InvalidHandle`] if the staging file behind `handle`
/// cannot be read or fails its hash verification, and [`GraphDocumentSideChannelError::Decode`]
/// for any error [`decode_graph_document_snapshot_packed`] returns for those bytes.
pub fn load_graph_document_side_channel(
    handle: &SideChannelHandle,
) -> Result<GraphDocumentSnapshot, GraphDocumentSideChannelError> {
    let file = az_proto_core::read_verified_staging_file(handle)?;
    Ok(decode_graph_document_snapshot_packed(&file.bytes)?)
}

#[derive(Debug, Error)]
pub enum GraphDocumentSideChannelError {
    #[error("invalid graph document side-channel handle: {0}")]
    InvalidHandle(#[from] StagingFileSideChannelError),

    #[error("failed to decode graph document snapshot: {0}")]
    Decode(#[from] Error),
}

fn write_graph_document_snapshot(
    snapshot: &GraphDocumentSnapshot,
    mut builder: project_capnp::graph_document_snapshot::Builder<'_>,
) -> Result<(), Error> {
    validate_graph_document_snapshot_for_transport(snapshot)?;
    builder.set_snapshot_version(GRAPH_DOCUMENT_SNAPSHOT_VERSION);
    write_document_id(
        snapshot.document_id.as_str(),
        builder.reborrow().init_document(),
    );
    write_revision(snapshot.revision.0, builder.reborrow().init_revision());
    write_visual_graph_document(&snapshot.document, builder.init_graph())?;
    Ok(())
}

fn read_graph_document_snapshot(
    reader: project_capnp::graph_document_snapshot::Reader<'_>,
) -> Result<GraphDocumentSnapshot, Error> {
    let snapshot_version = reader.get_snapshot_version();
    if snapshot_version != GRAPH_DOCUMENT_SNAPSHOT_VERSION {
        return Err(Error::failed(format!(
            "unsupported graph document snapshot version {snapshot_version}; expected {GRAPH_DOCUMENT_SNAPSHOT_VERSION}"
        )));
    }
    let snapshot = GraphDocumentSnapshot {
        document_id: DocumentId::new(read_document_id(reader.get_document()?)?),
        revision: DocumentRevision::new(read_revision(reader.get_revision()?)),
        document: read_visual_graph_document(reader.get_graph()?)?,
    };
    validate_graph_document_snapshot_for_transport(&snapshot)?;
    Ok(snapshot)
}

fn write_graph_command_batch_snapshot(
    batch: &GraphCommandBatchSnapshot,
    mut builder: project_capnp::graph_command_batch_snapshot::Builder<'_>,
) -> Result<(), Error> {
    validate_graph_command_batch_snapshot_for_transport(batch)?;
    builder.set_snapshot_version(GRAPH_COMMAND_BATCH_SNAPSHOT_VERSION);
    write_document_id(
        batch.document_id.as_str(),
        builder.reborrow().init_document(),
    );
    builder.set_client_batch_id(&batch.client_batch_id);
    if let Some(revision) = batch.expected_revision {
        write_revision(revision.0, builder.reborrow().init_expected_revision());
    } else {
        builder.set_no_expected_revision(());
    }

    let mut commands = builder
        .reborrow()
        .init_commands(capnp_list_index(batch.commands.len())?);
    for (index, command) in batch.commands.iter().enumerate() {
        write_graph_command(command, commands.reborrow().get(capnp_list_index(index)?))?;
    }
    Ok(())
}

fn read_graph_command_batch_snapshot(
    reader: project_capnp::graph_command_batch_snapshot::Reader<'_>,
) -> Result<GraphCommandBatchSnapshot, Error> {
    let snapshot_version = reader.get_snapshot_version();
    if snapshot_version != GRAPH_COMMAND_BATCH_SNAPSHOT_VERSION {
        return Err(Error::failed(format!(
            "unsupported graph command batch snapshot version {snapshot_version}; expected {GRAPH_COMMAND_BATCH_SNAPSHOT_VERSION}"
        )));
    }
    let expected_revision = match reader.which()? {
        project_capnp::graph_command_batch_snapshot::Which::NoExpectedRevision(()) => None,
        project_capnp::graph_command_batch_snapshot::Which::ExpectedRevision(revision) => {
            Some(DocumentRevision::new(read_revision(revision?)))
        }
    };
    let batch = GraphCommandBatchSnapshot {
        document_id: DocumentId::new(read_document_id(reader.get_document()?)?),
        expected_revision,
        client_batch_id: reader.get_client_batch_id()?.to_string()?,
        commands: reader
            .get_commands()?
            .iter()
            .map(read_graph_command)
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_graph_command_batch_snapshot_for_transport(&batch)?;
    Ok(batch)
}

fn write_graph_command_status_snapshot(
    status: &GraphCommandStatusSnapshot,
    mut builder: project_capnp::graph_command_status_snapshot::Builder<'_>,
) -> Result<(), Error> {
    validate_graph_command_status_snapshot_for_transport(status)?;
    builder.set_snapshot_version(GRAPH_COMMAND_STATUS_SNAPSHOT_VERSION);
    write_document_id(
        status.document_id.as_str(),
        builder.reborrow().init_document(),
    );
    builder.set_client_batch_id(&status.client_batch_id);
    builder.set_applied_command_count(status.applied_command_count);
    let mut diagnostics = builder
        .reborrow()
        .init_diagnostics(capnp_list_index(status.diagnostics.len())?);
    for (index, diagnostic) in status.diagnostics.iter().enumerate() {
        write_graph_command_diagnostic(
            diagnostic,
            diagnostics.reborrow().get(capnp_list_index(index)?),
        );
    }
    match &status.outcome {
        GraphCommandStatusOutcome::Accepted { revision } => {
            write_revision(revision.0, builder.reborrow().init_accepted_revision());
        }
        GraphCommandStatusOutcome::Rejected {
            command_index,
            reason,
        } => {
            let mut rejected = builder.reborrow().init_rejected();
            rejected.set_reason(reason);
            if let Some(command_index) = command_index {
                rejected.set_command_index(*command_index);
            } else {
                rejected.set_no_command_index(());
            }
        }
    }
    Ok(())
}

fn read_graph_command_status_snapshot(
    reader: project_capnp::graph_command_status_snapshot::Reader<'_>,
) -> Result<GraphCommandStatusSnapshot, Error> {
    let snapshot_version = reader.get_snapshot_version();
    if snapshot_version != GRAPH_COMMAND_STATUS_SNAPSHOT_VERSION {
        return Err(Error::failed(format!(
            "unsupported graph command status snapshot version {snapshot_version}; expected {GRAPH_COMMAND_STATUS_SNAPSHOT_VERSION}"
        )));
    }
    let outcome = match reader.which()? {
        project_capnp::graph_command_status_snapshot::Which::AcceptedRevision(revision) => {
            GraphCommandStatusOutcome::Accepted {
                revision: DocumentRevision::new(read_revision(revision?)),
            }
        }
        project_capnp::graph_command_status_snapshot::Which::Rejected(rejected) => {
            let rejected = rejected?;
            let command_index = match rejected.which()? {
                project_capnp::graph_command_rejection::Which::NoCommandIndex(()) => None,
                project_capnp::graph_command_rejection::Which::CommandIndex(value) => Some(value),
            };
            GraphCommandStatusOutcome::Rejected {
                command_index,
                reason: rejected.get_reason()?.to_string()?,
            }
        }
    };
    let status = GraphCommandStatusSnapshot {
        document_id: DocumentId::new(read_document_id(reader.get_document()?)?),
        client_batch_id: reader.get_client_batch_id()?.to_string()?,
        applied_command_count: reader.get_applied_command_count(),
        diagnostics: reader
            .get_diagnostics()?
            .iter()
            .map(read_graph_command_diagnostic)
            .collect::<Result<Vec<_>, _>>()?,
        outcome,
    };
    validate_graph_command_status_snapshot_for_transport(&status)?;
    Ok(status)
}

fn write_gamedata_catalog_snapshot(
    catalog: &GameDataCatalogSnapshot,
    mut builder: project_capnp::game_data_catalog_snapshot::Builder<'_>,
) -> Result<(), Error> {
    validate_gamedata_catalog_snapshot_for_transport(catalog)?;
    builder.set_catalog_version(catalog.catalog_version);
    builder.set_generated_unix_ms(catalog.generated_unix_ms);

    let mut tables = builder
        .reborrow()
        .init_tables(capnp_list_index(catalog.tables.len())?);
    for (index, table) in catalog.tables.iter().enumerate() {
        write_gamedata_table_descriptor(table, tables.reborrow().get(capnp_list_index(index)?))?;
    }

    let mut families = builder
        .reborrow()
        .init_families(capnp_list_index(catalog.families.len())?);
    for (index, family) in catalog.families.iter().enumerate() {
        write_gamedata_table_family_descriptor(
            family,
            families.reborrow().get(capnp_list_index(index)?),
        )?;
    }

    let mut managers = builder
        .reborrow()
        .init_managers(capnp_list_index(catalog.managers.len())?);
    for (index, manager) in catalog.managers.iter().enumerate() {
        write_gamedata_manager_catalog_entry(
            manager,
            managers.reborrow().get(capnp_list_index(index)?),
        )?;
    }

    let mut diagnostics = builder
        .reborrow()
        .init_diagnostics(capnp_list_index(catalog.diagnostics.len())?);
    for (index, diagnostic) in catalog.diagnostics.iter().enumerate() {
        write_gamedata_catalog_diagnostic(
            diagnostic,
            diagnostics.reborrow().get(capnp_list_index(index)?),
        );
    }
    Ok(())
}

fn read_gamedata_catalog_snapshot(
    reader: project_capnp::game_data_catalog_snapshot::Reader<'_>,
) -> Result<GameDataCatalogSnapshot, Error> {
    let catalog_version = reader.get_catalog_version();
    if catalog_version != GAMEDATA_CATALOG_SNAPSHOT_VERSION {
        return Err(Error::failed(format!(
            "unsupported GameData catalog snapshot version {catalog_version}; expected {GAMEDATA_CATALOG_SNAPSHOT_VERSION}"
        )));
    }

    let catalog = GameDataCatalogSnapshot {
        catalog_version,
        generated_unix_ms: reader.get_generated_unix_ms(),
        tables: reader
            .get_tables()?
            .iter()
            .map(read_gamedata_table_descriptor)
            .collect::<Result<Vec<_>, _>>()?,
        families: reader
            .get_families()?
            .iter()
            .map(read_gamedata_table_family_descriptor)
            .collect::<Result<Vec<_>, _>>()?,
        managers: reader
            .get_managers()?
            .iter()
            .map(read_gamedata_manager_catalog_entry)
            .collect::<Result<Vec<_>, _>>()?,
        diagnostics: reader
            .get_diagnostics()?
            .iter()
            .map(read_gamedata_catalog_diagnostic)
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_gamedata_catalog_snapshot_for_transport(&catalog)?;
    Ok(catalog)
}

fn write_optional_u64(
    value: Option<u64>,
    mut builder: core::core_capnp::optional_u64::Builder<'_>,
) {
    match value {
        Some(value) => builder.set_value(value),
        None => builder.set_none(()),
    }
}

fn read_optional_u64(
    reader: core::core_capnp::optional_u64::Reader<'_>,
) -> Result<Option<u64>, Error> {
    match reader.which()? {
        core::core_capnp::optional_u64::Which::None(()) => Ok(None),
        core::core_capnp::optional_u64::Which::Value(value) => Ok(Some(value)),
    }
}

fn write_gamedata_table_descriptor(
    table: &GameDataTableDescriptor,
    mut builder: project_capnp::game_data_table_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_name(&table.name);
    builder.set_row_type(&table.row_type);
    builder.set_source_root(&table.source_root);
    builder.set_source_path(&table.source_path);
    builder.set_owner(&table.owner);
    write_optional_u64(table.schema_hash, builder.reborrow().init_schema_hash());
    builder.set_document_id(&table.document_id);
    builder.set_schema_type(&table.schema_type);
    builder.set_category(&table.category);
    write_optional_u64(table.row_count, builder.reborrow().init_row_count());
    write_text_list(
        &table.families,
        builder
            .reborrow()
            .init_families(capnp_list_index(table.families.len())?),
    );
    table.source_ref.to_capnp(builder.init_source_ref())?;
    Ok(())
}

fn read_gamedata_table_descriptor(
    reader: project_capnp::game_data_table_descriptor::Reader<'_>,
) -> Result<GameDataTableDescriptor, Error> {
    Ok(GameDataTableDescriptor {
        name: reader.get_name()?.to_string()?,
        row_type: reader.get_row_type()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        owner: reader.get_owner()?.to_string()?,
        schema_hash: read_optional_u64(reader.get_schema_hash()?)?,
        document_id: reader.get_document_id()?.to_string()?,
        schema_type: reader.get_schema_type()?.to_string()?,
        category: reader.get_category()?.to_string()?,
        row_count: read_optional_u64(reader.get_row_count()?)?,
        families: read_text_list(reader.get_families()?)?,
        source_ref: WorkspaceSourceFileRef::from_capnp(reader.get_source_ref()?)?,
    })
}

fn write_gamedata_table_family_descriptor(
    family: &GameDataTableFamilyDescriptor,
    mut builder: project_capnp::game_data_table_family_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_name(&family.name);
    builder.set_row_type(&family.row_type);
    builder.set_owner(&family.owner);
    builder.set_duplicate_key_policy(&family.duplicate_key_policy);
    write_text_list(
        &family.tables,
        builder
            .reborrow()
            .init_tables(capnp_list_index(family.tables.len())?),
    );
    Ok(())
}

fn read_gamedata_table_family_descriptor(
    reader: project_capnp::game_data_table_family_descriptor::Reader<'_>,
) -> Result<GameDataTableFamilyDescriptor, Error> {
    Ok(GameDataTableFamilyDescriptor {
        name: reader.get_name()?.to_string()?,
        row_type: reader.get_row_type()?.to_string()?,
        owner: reader.get_owner()?.to_string()?,
        duplicate_key_policy: reader.get_duplicate_key_policy()?.to_string()?,
        tables: read_text_list(reader.get_tables()?)?,
    })
}

fn write_gamedata_provider_target(
    target: &GameDataProviderTarget,
    mut builder: project_capnp::game_data_provider_target::Builder<'_>,
) {
    builder.set_kind(&target.kind);
    builder.set_name(&target.name);
    builder.set_row_type(&target.row_type);
}

fn read_gamedata_provider_target(
    reader: project_capnp::game_data_provider_target::Reader<'_>,
) -> Result<GameDataProviderTarget, Error> {
    Ok(GameDataProviderTarget {
        kind: reader.get_kind()?.to_string()?,
        name: reader.get_name()?.to_string()?,
        row_type: reader.get_row_type()?.to_string()?,
    })
}

fn write_gamedata_manager_node_ref(
    node: &GameDataManagerNodeRef,
    mut builder: project_capnp::game_data_manager_node_ref::Builder<'_>,
) {
    builder.set_key(&node.key);
    builder.set_label(&node.label);
    builder.set_kind(&node.kind);
}

fn read_gamedata_manager_node_ref(
    reader: project_capnp::game_data_manager_node_ref::Reader<'_>,
) -> Result<GameDataManagerNodeRef, Error> {
    Ok(GameDataManagerNodeRef {
        key: reader.get_key()?.to_string()?,
        label: reader.get_label()?.to_string()?,
        kind: reader.get_kind()?.to_string()?,
    })
}

fn write_gamedata_key_policy(
    policy: &GameDataKeyPolicy,
    mut builder: project_capnp::game_data_key_policy::Builder<'_>,
) -> Result<(), Error> {
    builder.set_kind(&policy.kind);
    write_text_list(
        &policy.transforms,
        builder
            .reborrow()
            .init_transforms(capnp_list_index(policy.transforms.len())?),
    );
    builder.set_reject_zero_crc(policy.reject_zero_crc);
    builder.set_store_key_text(policy.store_key_text);
    Ok(())
}

fn read_gamedata_key_policy(
    reader: project_capnp::game_data_key_policy::Reader<'_>,
) -> Result<GameDataKeyPolicy, Error> {
    Ok(GameDataKeyPolicy {
        kind: reader.get_kind()?.to_string()?,
        transforms: read_text_list(reader.get_transforms()?)?,
        reject_zero_crc: reader.get_reject_zero_crc(),
        store_key_text: reader.get_store_key_text(),
    })
}

fn write_gamedata_manager_input(
    input: &GameDataManagerInput,
    mut builder: project_capnp::game_data_manager_input::Builder<'_>,
) {
    builder.set_kind(&input.kind);
    builder.set_name(&input.name);
    builder.set_row_type(&input.row_type);
    builder.set_source_root(&input.source_root);
    builder.set_source_path(&input.source_path);
    builder.set_detail(&input.detail);
    builder.set_provider_kind(&input.provider_kind);
}

fn read_gamedata_manager_input(
    reader: project_capnp::game_data_manager_input::Reader<'_>,
) -> Result<GameDataManagerInput, Error> {
    Ok(GameDataManagerInput {
        kind: reader.get_kind()?.to_string()?,
        name: reader.get_name()?.to_string()?,
        row_type: reader.get_row_type()?.to_string()?,
        source_root: reader.get_source_root()?.to_string()?,
        source_path: reader.get_source_path()?.to_string()?,
        detail: reader.get_detail()?.to_string()?,
        provider_kind: reader.get_provider_kind()?.to_string()?,
    })
}

fn write_gamedata_row_filter(
    filter: &GameDataRowFilter,
    mut builder: project_capnp::game_data_row_filter::Builder<'_>,
) {
    builder.set_field(&filter.field);
    builder.set_predicate(&filter.predicate);
    builder.set_compare_field(&filter.compare_field);
}

fn read_gamedata_row_filter(
    reader: project_capnp::game_data_row_filter::Reader<'_>,
) -> Result<GameDataRowFilter, Error> {
    Ok(GameDataRowFilter {
        field: reader.get_field()?.to_string()?,
        predicate: reader.get_predicate()?.to_string()?,
        compare_field: reader.get_compare_field()?.to_string()?,
    })
}

fn write_gamedata_projection_transform(
    transform: &GameDataProjectionTransform,
    mut builder: project_capnp::game_data_projection_transform::Builder<'_>,
) {
    builder.set_field(&transform.field);
    builder.set_source_column(&transform.source_column);
    builder.set_kind(&transform.kind);
}

fn read_gamedata_projection_transform(
    reader: project_capnp::game_data_projection_transform::Reader<'_>,
) -> Result<GameDataProjectionTransform, Error> {
    Ok(GameDataProjectionTransform {
        field: reader.get_field()?.to_string()?,
        source_column: reader.get_source_column()?.to_string()?,
        kind: reader.get_kind()?.to_string()?,
    })
}

fn write_gamedata_secondary_index(
    index: &GameDataSecondaryIndex,
    mut builder: project_capnp::game_data_secondary_index::Builder<'_>,
) {
    builder.set_name(&index.name);
    builder.set_field(&index.field);
    builder.set_key_kind(&index.key_kind);
    builder.set_storage(&index.storage);
    builder.set_duplicate_key_policy(&index.duplicate_key_policy);
}

fn read_gamedata_secondary_index(
    reader: project_capnp::game_data_secondary_index::Reader<'_>,
) -> Result<GameDataSecondaryIndex, Error> {
    Ok(GameDataSecondaryIndex {
        name: reader.get_name()?.to_string()?,
        field: reader.get_field()?.to_string()?,
        key_kind: reader.get_key_kind()?.to_string()?,
        storage: reader.get_storage()?.to_string()?,
        duplicate_key_policy: reader.get_duplicate_key_policy()?.to_string()?,
    })
}

fn write_gamedata_catalog_diagnostic(
    diagnostic: &GameDataCatalogDiagnostic,
    mut builder: project_capnp::game_data_catalog_diagnostic::Builder<'_>,
) {
    builder.set_code(&diagnostic.code);
    builder.set_message(&diagnostic.message);
    builder.set_target_key(&diagnostic.target_key);
    builder.set_target_label(&diagnostic.target_label);
}

fn read_gamedata_catalog_diagnostic(
    reader: project_capnp::game_data_catalog_diagnostic::Reader<'_>,
) -> Result<GameDataCatalogDiagnostic, Error> {
    Ok(GameDataCatalogDiagnostic {
        code: reader.get_code()?.to_string()?,
        message: reader.get_message()?.to_string()?,
        target_key: reader.get_target_key()?.to_string()?,
        target_label: reader.get_target_label()?.to_string()?,
    })
}

fn write_gamedata_manager_catalog_entry(
    entry: &GameDataManagerCatalogEntry,
    mut builder: project_capnp::game_data_manager_catalog_entry::Builder<'_>,
) -> Result<(), Error> {
    builder.set_key(&entry.key);
    builder.set_name(&entry.name);
    builder.set_owner(&entry.owner);
    builder.set_row_type(&entry.row_type);
    builder.set_kind(&entry.kind);
    builder.set_output_type(&entry.output_type);
    builder.set_read_only(entry.read_only);
    if let Some(target) = &entry.provider_target {
        write_gamedata_provider_target(target, builder.reborrow().init_provider_target());
    }
    write_gamedata_key_policy(&entry.key_policy, builder.reborrow().init_key_policy())?;
    builder.set_duplicate_key_policy(&entry.duplicate_key_policy);

    let mut inputs = builder
        .reborrow()
        .init_inputs(capnp_list_index(entry.inputs.len())?);
    for (index, input) in entry.inputs.iter().enumerate() {
        write_gamedata_manager_input(input, inputs.reborrow().get(capnp_list_index(index)?));
    }

    let mut filters = builder
        .reborrow()
        .init_row_filters(capnp_list_index(entry.row_filters.len())?);
    for (index, filter) in entry.row_filters.iter().enumerate() {
        write_gamedata_row_filter(filter, filters.reborrow().get(capnp_list_index(index)?));
    }

    let mut transforms = builder
        .reborrow()
        .init_projection_transforms(capnp_list_index(entry.projection_transforms.len())?);
    for (index, transform) in entry.projection_transforms.iter().enumerate() {
        write_gamedata_projection_transform(
            transform,
            transforms.reborrow().get(capnp_list_index(index)?),
        );
    }

    let mut indexes = builder
        .reborrow()
        .init_secondary_indexes(capnp_list_index(entry.secondary_indexes.len())?);
    for (index, secondary_index) in entry.secondary_indexes.iter().enumerate() {
        write_gamedata_secondary_index(
            secondary_index,
            indexes.reborrow().get(capnp_list_index(index)?),
        );
    }

    let mut source_targets = builder
        .reborrow()
        .init_source_targets(capnp_list_index(entry.source_targets.len())?);
    for (index, target) in entry.source_targets.iter().enumerate() {
        write_gamedata_provider_target(
            target,
            source_targets.reborrow().get(capnp_list_index(index)?),
        );
    }

    let mut dependencies = builder
        .reborrow()
        .init_dependencies(capnp_list_index(entry.dependencies.len())?);
    for (index, dependency) in entry.dependencies.iter().enumerate() {
        write_gamedata_manager_node_ref(
            dependency,
            dependencies.reborrow().get(capnp_list_index(index)?),
        );
    }

    let mut dependents = builder
        .reborrow()
        .init_dependents(capnp_list_index(entry.dependents.len())?);
    for (index, dependent) in entry.dependents.iter().enumerate() {
        write_gamedata_manager_node_ref(
            dependent,
            dependents.reborrow().get(capnp_list_index(index)?),
        );
    }

    let mut diagnostics = builder
        .reborrow()
        .init_diagnostics(capnp_list_index(entry.diagnostics.len())?);
    for (index, diagnostic) in entry.diagnostics.iter().enumerate() {
        write_gamedata_catalog_diagnostic(
            diagnostic,
            diagnostics.reborrow().get(capnp_list_index(index)?),
        );
    }

    if entry.projection_hash.len() != blake3::OUT_LEN {
        return Err(invalid_project_protocol_value(
            "GameData manager catalog entry",
            format!(
                "projection hash for `{}` must have {} bytes, got {}",
                entry.key,
                blake3::OUT_LEN,
                entry.projection_hash.len()
            ),
        ));
    }
    builder.set_projection_hash(&entry.projection_hash);
    Ok(())
}

fn read_gamedata_manager_catalog_entry(
    reader: project_capnp::game_data_manager_catalog_entry::Reader<'_>,
) -> Result<GameDataManagerCatalogEntry, Error> {
    Ok(GameDataManagerCatalogEntry {
        key: reader.get_key()?.to_string()?,
        name: reader.get_name()?.to_string()?,
        owner: reader.get_owner()?.to_string()?,
        row_type: reader.get_row_type()?.to_string()?,
        kind: reader.get_kind()?.to_string()?,
        output_type: reader.get_output_type()?.to_string()?,
        read_only: reader.get_read_only(),
        provider_target: if reader.has_provider_target() {
            Some(read_gamedata_provider_target(
                reader.get_provider_target()?,
            )?)
        } else {
            None
        },
        key_policy: read_gamedata_key_policy(reader.get_key_policy()?)?,
        duplicate_key_policy: reader.get_duplicate_key_policy()?.to_string()?,
        inputs: reader
            .get_inputs()?
            .iter()
            .map(read_gamedata_manager_input)
            .collect::<Result<Vec<_>, _>>()?,
        row_filters: reader
            .get_row_filters()?
            .iter()
            .map(read_gamedata_row_filter)
            .collect::<Result<Vec<_>, _>>()?,
        projection_transforms: reader
            .get_projection_transforms()?
            .iter()
            .map(read_gamedata_projection_transform)
            .collect::<Result<Vec<_>, _>>()?,
        secondary_indexes: reader
            .get_secondary_indexes()?
            .iter()
            .map(read_gamedata_secondary_index)
            .collect::<Result<Vec<_>, _>>()?,
        source_targets: reader
            .get_source_targets()?
            .iter()
            .map(read_gamedata_provider_target)
            .collect::<Result<Vec<_>, _>>()?,
        dependencies: reader
            .get_dependencies()?
            .iter()
            .map(read_gamedata_manager_node_ref)
            .collect::<Result<Vec<_>, _>>()?,
        dependents: reader
            .get_dependents()?
            .iter()
            .map(read_gamedata_manager_node_ref)
            .collect::<Result<Vec<_>, _>>()?,
        diagnostics: reader
            .get_diagnostics()?
            .iter()
            .map(read_gamedata_catalog_diagnostic)
            .collect::<Result<Vec<_>, _>>()?,
        projection_hash: reader.get_projection_hash()?.to_vec(),
    })
}

fn write_node_type_catalog_snapshot(
    catalog: &NodeTypeCatalog,
    mut builder: project_capnp::node_type_catalog_snapshot::Builder<'_>,
) -> Result<(), Error> {
    builder.set_catalog_version(catalog.catalog_version);
    builder.set_generated_unix_ms(catalog.generated_unix_ms);

    let mut node_types = builder
        .reborrow()
        .init_node_types(capnp_list_index(catalog.node_types.len())?);
    for (index, node_type) in catalog.node_types.iter().enumerate() {
        write_node_type_descriptor(
            node_type,
            node_types.reborrow().get(capnp_list_index(index)?),
        )?;
    }
    Ok(())
}

fn read_node_type_catalog_snapshot(
    reader: project_capnp::node_type_catalog_snapshot::Reader<'_>,
) -> Result<NodeTypeCatalog, Error> {
    let catalog_version = reader.get_catalog_version();
    if catalog_version != NODE_TYPE_CATALOG_SNAPSHOT_VERSION {
        return Err(Error::failed(format!(
            "unsupported node type catalog snapshot version {catalog_version}; expected {NODE_TYPE_CATALOG_SNAPSHOT_VERSION}"
        )));
    }

    let node_types = reader
        .get_node_types()?
        .iter()
        .map(read_node_type_descriptor)
        .collect::<Result<Vec<_>, _>>()?;

    NodeTypeCatalog::try_new(catalog_version, reader.get_generated_unix_ms(), node_types)
        .map_err(|error| Error::failed(format!("invalid node type catalog snapshot: {error}")))
}

fn write_node_type_descriptor(
    node_type: &NodeTypeDescriptor,
    mut builder: project_capnp::node_type_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_id(node_type.id.as_str());
    builder.set_version(node_type.version);
    builder.set_display_name(&node_type.display_name);
    write_text_list(
        &node_type.category_path,
        builder
            .reborrow()
            .init_category_path(capnp_list_index(node_type.category_path.len())?),
    );
    if let Some(description) = &node_type.description {
        builder.set_description(description);
    }

    let mut ports = builder
        .reborrow()
        .init_ports(capnp_list_index(node_type.ports.len())?);
    for (index, port) in node_type.ports.iter().enumerate() {
        write_node_port_descriptor(port, ports.reborrow().get(capnp_list_index(index)?))?;
    }

    let mut capabilities = builder
        .reborrow()
        .init_capabilities(capnp_list_index(node_type.capabilities.len())?);
    for (index, capability) in node_type.capabilities.iter().enumerate() {
        write_node_capability(
            capability,
            capabilities.reborrow().get(capnp_list_index(index)?),
        )?;
    }

    let mut source_links = builder
        .reborrow()
        .init_source_links(capnp_list_index(node_type.source_links.len())?);
    for (index, source_link) in node_type.source_links.iter().enumerate() {
        write_node_source_link(
            source_link,
            source_links.reborrow().get(capnp_list_index(index)?),
        );
    }

    write_text_list(
        &node_type.tags,
        builder
            .reborrow()
            .init_tags(capnp_list_index(node_type.tags.len())?),
    );

    if let Some(binding) = &node_type.runtime_binding {
        write_node_runtime_binding(binding, builder.init_runtime_binding())?;
    } else {
        builder.set_no_runtime_binding(());
    }
    Ok(())
}

fn read_node_type_descriptor(
    reader: project_capnp::node_type_descriptor::Reader<'_>,
) -> Result<NodeTypeDescriptor, Error> {
    let runtime_binding = match reader.which()? {
        project_capnp::node_type_descriptor::Which::NoRuntimeBinding(()) => None,
        project_capnp::node_type_descriptor::Which::RuntimeBinding(binding) => {
            Some(read_node_runtime_binding(binding?)?)
        }
    };

    Ok(NodeTypeDescriptor {
        id: NodeTypeId::new(reader.get_id()?.to_string()?),
        version: reader.get_version(),
        display_name: reader.get_display_name()?.to_string()?,
        category_path: read_text_list(reader.get_category_path()?)?,
        description: optional_text(reader.has_description(), reader.get_description())?,
        ports: reader
            .get_ports()?
            .iter()
            .map(read_node_port_descriptor)
            .collect::<Result<Vec<_>, _>>()?,
        capabilities: reader
            .get_capabilities()?
            .iter()
            .map(read_node_capability)
            .collect::<Result<Vec<_>, _>>()?,
        runtime_binding,
        source_links: reader
            .get_source_links()?
            .iter()
            .map(read_node_source_link)
            .collect::<Result<Vec<_>, _>>()?,
        tags: read_text_list(reader.get_tags()?)?,
    })
}

fn write_node_port_descriptor(
    port: &NodePortDescriptor,
    mut builder: project_capnp::node_port_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_id(port.id.0);
    builder.set_name(&port.name);
    builder.set_direction(match port.direction {
        NodePortDirection::Input => project_capnp::NodePortDirection::Input,
        NodePortDirection::Output => project_capnp::NodePortDirection::Output,
    });
    write_node_port_value(&port.value, builder.reborrow().init_value())?;
    builder.set_capacity(match port.capacity {
        NodePortCapacity::Single => project_capnp::NodePortCapacity::Single,
        NodePortCapacity::Multiple => project_capnp::NodePortCapacity::Multiple,
    });
    if let Some(description) = &port.description {
        builder.set_description(description);
    }
    if let Some(default_value) = &port.default_value {
        default_value.to_capnp(builder.reborrow().init_default_value())?;
    } else {
        builder.reborrow().set_no_default_value(());
    }
    write_node_port_layout(port.layout, builder.init_layout())?;
    Ok(())
}

fn read_node_port_descriptor(
    reader: project_capnp::node_port_descriptor::Reader<'_>,
) -> Result<NodePortDescriptor, Error> {
    let default_value = match reader.which()? {
        project_capnp::node_port_descriptor::Which::NoDefaultValue(()) => None,
        project_capnp::node_port_descriptor::Which::DefaultValue(value) => {
            Some(vnext::ReflectedValueEnvelope::from_capnp(value?)?)
        }
    };
    let direction = match reader.get_direction()? {
        project_capnp::NodePortDirection::Input => NodePortDirection::Input,
        project_capnp::NodePortDirection::Output => NodePortDirection::Output,
    };
    Ok(NodePortDescriptor {
        id: NodePortId(reader.get_id()),
        name: reader.get_name()?.to_string()?,
        direction,
        value: read_node_port_value(reader.get_value()?)?,
        capacity: match reader.get_capacity()? {
            project_capnp::NodePortCapacity::Single => NodePortCapacity::Single,
            project_capnp::NodePortCapacity::Multiple => NodePortCapacity::Multiple,
        },
        description: optional_text(reader.has_description(), reader.get_description())?,
        default_value,
        layout: if reader.has_layout() {
            read_node_port_layout(reader.get_layout()?)?
        } else {
            direction.default_layout()
        },
    })
}

fn write_node_port_layout(
    layout: NodePortLayout,
    mut builder: project_capnp::node_port_layout::Builder<'_>,
) -> Result<(), Error> {
    builder.set_side(match layout.side {
        NodePortSide::North => project_capnp::NodePortSide::North,
        NodePortSide::East => project_capnp::NodePortSide::East,
        NodePortSide::South => project_capnp::NodePortSide::South,
        NodePortSide::West => project_capnp::NodePortSide::West,
    });
    if let Some(order) = layout.order {
        builder.set_order(order);
    } else {
        builder.set_no_order(());
    }
    match layout.attachment {
        NodePortAttachment::EvenlySpaced => {
            builder.set_attachment(project_capnp::NodePortAttachment::EvenlySpaced);
            builder.set_fixed_fraction_per_mille(0);
        }
        NodePortAttachment::FixedFraction { per_mille } => {
            if per_mille > 1000 {
                return Err(Error::failed(format!(
                    "node port layout fixed attachment {per_mille} exceeds 1000"
                )));
            }
            builder.set_attachment(project_capnp::NodePortAttachment::FixedFraction);
            builder.set_fixed_fraction_per_mille(per_mille);
        }
    }
    Ok(())
}

fn read_node_port_layout(
    reader: project_capnp::node_port_layout::Reader<'_>,
) -> Result<NodePortLayout, Error> {
    let order = match reader.which()? {
        project_capnp::node_port_layout::Which::NoOrder(()) => None,
        project_capnp::node_port_layout::Which::Order(order) => Some(order),
    };
    let attachment = match reader.get_attachment()? {
        project_capnp::NodePortAttachment::EvenlySpaced => NodePortAttachment::EvenlySpaced,
        project_capnp::NodePortAttachment::FixedFraction => {
            let per_mille = reader.get_fixed_fraction_per_mille();
            if per_mille > 1000 {
                return Err(Error::failed(format!(
                    "node port layout fixed attachment {per_mille} exceeds 1000"
                )));
            }
            NodePortAttachment::FixedFraction { per_mille }
        }
    };
    Ok(NodePortLayout {
        side: match reader.get_side()? {
            project_capnp::NodePortSide::North => NodePortSide::North,
            project_capnp::NodePortSide::East => NodePortSide::East,
            project_capnp::NodePortSide::South => NodePortSide::South,
            project_capnp::NodePortSide::West => NodePortSide::West,
        },
        order,
        attachment,
    })
}

fn write_node_port_value(
    value: &NodePortValue,
    mut builder: project_capnp::node_port_value::Builder<'_>,
) -> Result<(), Error> {
    match value {
        NodePortValue::Execution => builder.set_execution(()),
        NodePortValue::Data { schema_type } => {
            builder.init_data().set_schema_type(schema_type);
        }
        NodePortValue::DynamicData {
            group,
            accepted_schema_types,
        } => {
            let mut dynamic = builder.init_dynamic_data();
            dynamic.set_group(group);
            write_text_list(
                accepted_schema_types,
                dynamic.init_accepted_schema_types(capnp_list_index(accepted_schema_types.len())?),
            );
        }
    }
    Ok(())
}

fn read_node_port_value(
    reader: project_capnp::node_port_value::Reader<'_>,
) -> Result<NodePortValue, Error> {
    match reader.which()? {
        project_capnp::node_port_value::Which::Execution(()) => Ok(NodePortValue::Execution),
        project_capnp::node_port_value::Which::Data(data) => {
            let data = data?;
            Ok(NodePortValue::Data {
                schema_type: data.get_schema_type()?.to_string()?,
            })
        }
        project_capnp::node_port_value::Which::DynamicData(dynamic) => {
            let dynamic = dynamic?;
            Ok(NodePortValue::DynamicData {
                group: dynamic.get_group()?.to_string()?,
                accepted_schema_types: read_text_list(dynamic.get_accepted_schema_types()?)?,
            })
        }
    }
}

fn write_node_capability(
    capability: &NodeCapability,
    mut builder: project_capnp::node_capability::Builder<'_>,
) -> Result<(), Error> {
    builder.set_id(&capability.id);
    write_text_list(
        &capability.markers,
        builder.init_markers(capnp_list_index(capability.markers.len())?),
    );
    Ok(())
}

fn read_node_capability(
    reader: project_capnp::node_capability::Reader<'_>,
) -> Result<NodeCapability, Error> {
    Ok(NodeCapability {
        id: reader.get_id()?.to_string()?,
        markers: read_text_list(reader.get_markers()?)?,
    })
}

fn write_node_runtime_binding(
    binding: &NodeRuntimeBinding,
    builder: project_capnp::node_runtime_binding::Builder<'_>,
) -> Result<(), Error> {
    match binding {
        NodeRuntimeBinding::RustSymbol {
            package,
            symbol,
            call_abi,
        } => {
            let mut rust_symbol = builder.init_rust_symbol();
            rust_symbol.set_package(package);
            rust_symbol.set_symbol(symbol);
            write_rust_node_call_abi(call_abi, rust_symbol.init_call_abi())?;
        }
        NodeRuntimeBinding::AssetBuilder { builder_id } => {
            builder.init_asset_builder().set_builder_id(builder_id);
        }
        NodeRuntimeBinding::RuntimeComponent { component_type } => {
            builder
                .init_runtime_component()
                .set_component_type(component_type);
        }
        NodeRuntimeBinding::External { kind, locator } => {
            let mut external = builder.init_external();
            external.set_kind(kind);
            external.set_locator(locator);
        }
    }
    Ok(())
}

fn read_node_runtime_binding(
    reader: project_capnp::node_runtime_binding::Reader<'_>,
) -> Result<NodeRuntimeBinding, Error> {
    match reader.which()? {
        project_capnp::node_runtime_binding::Which::RustSymbol(value) => {
            let value = value?;
            Ok(NodeRuntimeBinding::RustSymbol {
                package: value.get_package()?.to_string()?,
                symbol: value.get_symbol()?.to_string()?,
                call_abi: if value.has_call_abi() {
                    read_rust_node_call_abi(value.get_call_abi()?)?
                } else {
                    RustNodeCallAbi::ContextSchedule
                },
            })
        }
        project_capnp::node_runtime_binding::Which::AssetBuilder(value) => {
            let value = value?;
            Ok(NodeRuntimeBinding::AssetBuilder {
                builder_id: value.get_builder_id()?.to_string()?,
            })
        }
        project_capnp::node_runtime_binding::Which::RuntimeComponent(value) => {
            let value = value?;
            Ok(NodeRuntimeBinding::RuntimeComponent {
                component_type: value.get_component_type()?.to_string()?,
            })
        }
        project_capnp::node_runtime_binding::Which::External(value) => {
            let value = value?;
            Ok(NodeRuntimeBinding::External {
                kind: value.get_kind()?.to_string()?,
                locator: value.get_locator()?.to_string()?,
            })
        }
    }
}

fn write_rust_node_call_abi(
    call_abi: &RustNodeCallAbi,
    mut builder: project_capnp::rust_node_call_abi::Builder<'_>,
) -> Result<(), Error> {
    match call_abi {
        RustNodeCallAbi::ContextSchedule => builder.set_context_schedule(()),
        RustNodeCallAbi::TypedDataflow(dataflow) => {
            write_rust_typed_dataflow_node_call(dataflow, builder.init_typed_dataflow())?;
        }
    }
    Ok(())
}

fn read_rust_node_call_abi(
    reader: project_capnp::rust_node_call_abi::Reader<'_>,
) -> Result<RustNodeCallAbi, Error> {
    match reader.which()? {
        project_capnp::rust_node_call_abi::Which::ContextSchedule(()) => {
            Ok(RustNodeCallAbi::ContextSchedule)
        }
        project_capnp::rust_node_call_abi::Which::TypedDataflow(dataflow) => Ok(
            RustNodeCallAbi::TypedDataflow(read_rust_typed_dataflow_node_call(dataflow?)?),
        ),
    }
}

fn write_rust_typed_dataflow_node_call(
    dataflow: &RustTypedDataflowNodeCall,
    mut builder: project_capnp::rust_typed_dataflow_node_call::Builder<'_>,
) -> Result<(), Error> {
    let mut parameters = builder
        .reborrow()
        .init_parameters(capnp_list_index(dataflow.parameters.len())?);
    for (index, parameter) in dataflow.parameters.iter().enumerate() {
        write_rust_dataflow_parameter(
            parameter,
            parameters.reborrow().get(capnp_list_index(index)?),
        );
    }
    write_rust_dataflow_output(&dataflow.output, builder.reborrow().init_output())?;
    builder.set_result(match dataflow.result {
        RustCallResult::Plain => project_capnp::RustCallResult::Plain,
        RustCallResult::Result => project_capnp::RustCallResult::Result,
    });
    Ok(())
}

fn read_rust_typed_dataflow_node_call(
    reader: project_capnp::rust_typed_dataflow_node_call::Reader<'_>,
) -> Result<RustTypedDataflowNodeCall, Error> {
    Ok(RustTypedDataflowNodeCall {
        parameters: reader
            .get_parameters()?
            .iter()
            .map(read_rust_dataflow_parameter)
            .collect::<Result<Vec<_>, _>>()?,
        output: read_rust_dataflow_output(reader.get_output()?)?,
        result: match reader.get_result()? {
            project_capnp::RustCallResult::Plain => RustCallResult::Plain,
            project_capnp::RustCallResult::Result => RustCallResult::Result,
        },
    })
}

fn write_rust_dataflow_parameter(
    parameter: &RustDataflowParameter,
    mut builder: project_capnp::rust_dataflow_parameter::Builder<'_>,
) {
    match parameter.source {
        RustDataflowParameterSource::RuntimeContext => {
            builder.set_source(project_capnp::RustDataflowParameterSource::RuntimeContext);
            builder.set_input_port_id(0);
        }
        RustDataflowParameterSource::InputPort { port } => {
            builder.set_source(project_capnp::RustDataflowParameterSource::InputPort);
            builder.set_input_port_id(port.0);
        }
    }
    builder.set_rust_type(&parameter.rust_type);
    builder.set_passing(match parameter.passing {
        RustValuePassing::ByValue => project_capnp::RustValuePassing::ByValue,
        RustValuePassing::BySharedRef => project_capnp::RustValuePassing::BySharedRef,
        RustValuePassing::ByMutableRef => project_capnp::RustValuePassing::ByMutableRef,
    });
}

fn read_rust_dataflow_parameter(
    reader: project_capnp::rust_dataflow_parameter::Reader<'_>,
) -> Result<RustDataflowParameter, Error> {
    let source = match reader.get_source()? {
        project_capnp::RustDataflowParameterSource::RuntimeContext => {
            RustDataflowParameterSource::RuntimeContext
        }
        project_capnp::RustDataflowParameterSource::InputPort => {
            RustDataflowParameterSource::InputPort {
                port: NodePortId::new(reader.get_input_port_id()),
            }
        }
    };
    Ok(RustDataflowParameter {
        source,
        rust_type: reader.get_rust_type()?.to_string()?,
        passing: match reader.get_passing()? {
            project_capnp::RustValuePassing::ByValue => RustValuePassing::ByValue,
            project_capnp::RustValuePassing::BySharedRef => RustValuePassing::BySharedRef,
            project_capnp::RustValuePassing::ByMutableRef => RustValuePassing::ByMutableRef,
        },
    })
}

fn write_rust_dataflow_output(
    output: &RustDataflowOutput,
    mut builder: project_capnp::rust_dataflow_output::Builder<'_>,
) -> Result<(), Error> {
    match output {
        RustDataflowOutput::None => builder.set_none(()),
        RustDataflowOutput::Single { port, rust_type } => {
            let mut single = builder.init_single();
            single.set_port_id(port.0);
            single.set_rust_type(rust_type);
        }
        RustDataflowOutput::StructFields { rust_type, fields } => {
            let mut output = builder.init_struct_fields();
            output.set_rust_type(rust_type);
            let mut field_list = output.init_fields(capnp_list_index(fields.len())?);
            for (index, field) in fields.iter().enumerate() {
                write_rust_dataflow_output_field(
                    field,
                    field_list.reborrow().get(capnp_list_index(index)?),
                );
            }
        }
    }
    Ok(())
}

fn read_rust_dataflow_output(
    reader: project_capnp::rust_dataflow_output::Reader<'_>,
) -> Result<RustDataflowOutput, Error> {
    match reader.which()? {
        project_capnp::rust_dataflow_output::Which::None(()) => Ok(RustDataflowOutput::None),
        project_capnp::rust_dataflow_output::Which::Single(single) => {
            let single = single?;
            Ok(RustDataflowOutput::Single {
                port: NodePortId::new(single.get_port_id()),
                rust_type: single.get_rust_type()?.to_string()?,
            })
        }
        project_capnp::rust_dataflow_output::Which::StructFields(output) => {
            let output = output?;
            Ok(RustDataflowOutput::StructFields {
                rust_type: output.get_rust_type()?.to_string()?,
                fields: output
                    .get_fields()?
                    .iter()
                    .map(read_rust_dataflow_output_field)
                    .collect::<Result<Vec<_>, _>>()?,
            })
        }
    }
}

fn write_rust_dataflow_output_field(
    field: &RustDataflowOutputField,
    mut builder: project_capnp::rust_dataflow_output_field::Builder<'_>,
) {
    builder.set_port_id(field.port.0);
    builder.set_field(&field.field);
    builder.set_rust_type(&field.rust_type);
}

fn read_rust_dataflow_output_field(
    reader: project_capnp::rust_dataflow_output_field::Reader<'_>,
) -> Result<RustDataflowOutputField, Error> {
    Ok(RustDataflowOutputField {
        port: NodePortId::new(reader.get_port_id()),
        field: reader.get_field()?.to_string()?,
        rust_type: reader.get_rust_type()?.to_string()?,
    })
}

fn write_node_source_link(
    link: &NodeSourceLink,
    mut builder: project_capnp::node_source_link::Builder<'_>,
) {
    if let Some(package) = &link.package {
        builder.set_package(package);
    }
    if let Some(module_path) = &link.module_path {
        builder.set_module_path(module_path);
    }
    if let Some(symbol_path) = &link.symbol_path {
        builder.set_symbol_path(symbol_path);
    }
    if let Some(file) = &link.file {
        builder.set_file(file);
    }
    if let Some(line) = link.line {
        builder.set_line(line);
    }
    if let Some(column) = link.column {
        builder.set_column(column);
    }
    if let Some(docs_url) = &link.docs_url {
        builder.set_docs_url(docs_url);
    }
}

fn read_node_source_link(
    reader: project_capnp::node_source_link::Reader<'_>,
) -> Result<NodeSourceLink, Error> {
    Ok(NodeSourceLink {
        package: optional_text(reader.has_package(), reader.get_package())?,
        module_path: optional_text(reader.has_module_path(), reader.get_module_path())?,
        symbol_path: optional_text(reader.has_symbol_path(), reader.get_symbol_path())?,
        file: optional_text(reader.has_file(), reader.get_file())?,
        line: non_zero_u32_option(reader.get_line()),
        column: non_zero_u32_option(reader.get_column()),
        docs_url: optional_text(reader.has_docs_url(), reader.get_docs_url())?,
    })
}

fn write_graph_type_catalog_snapshot(
    catalog: &GraphTypeCatalog,
    mut builder: project_capnp::graph_type_catalog_snapshot::Builder<'_>,
) -> Result<(), Error> {
    builder.set_catalog_version(catalog.catalog_version);
    builder.set_generated_unix_ms(catalog.generated_unix_ms);

    let mut graph_types = builder
        .reborrow()
        .init_graph_types(capnp_list_index(catalog.graph_types.len())?);
    for (index, graph_type) in catalog.graph_types.iter().enumerate() {
        write_graph_type_descriptor(
            graph_type,
            graph_types.reborrow().get(capnp_list_index(index)?),
        )?;
    }
    Ok(())
}

fn read_graph_type_catalog_snapshot(
    reader: project_capnp::graph_type_catalog_snapshot::Reader<'_>,
) -> Result<GraphTypeCatalog, Error> {
    let catalog_version = reader.get_catalog_version();
    if catalog_version != GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION {
        return Err(Error::failed(format!(
            "unsupported graph type catalog snapshot version {catalog_version}; expected {GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION}"
        )));
    }

    let graph_types = reader
        .get_graph_types()?
        .iter()
        .map(read_graph_type_descriptor)
        .collect::<Result<Vec<_>, _>>()?;

    GraphTypeCatalog::try_new(catalog_version, reader.get_generated_unix_ms(), graph_types)
        .map_err(|error| Error::failed(format!("invalid graph type catalog snapshot: {error}")))
}

fn write_graph_type_descriptor(
    graph_type: &GraphTypeDescriptor,
    mut builder: project_capnp::graph_type_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_id(graph_type.id.as_str());
    builder.set_version(graph_type.version);
    builder.set_display_name(&graph_type.display_name);
    if let Some(description) = &graph_type.description {
        builder.set_description(description);
    }
    write_text_list(
        &graph_type.category_path,
        builder
            .reborrow()
            .init_category_path(capnp_list_index(graph_type.category_path.len())?),
    );
    write_graph_source_workflow(
        &graph_type.source_workflow,
        builder.reborrow().init_source_workflow(),
    );
    write_graph_document_template(&graph_type.template, builder.reborrow().init_template())?;

    let mut allowed_node_catalogs = builder
        .reborrow()
        .init_allowed_node_catalogs(capnp_list_index(graph_type.allowed_node_catalogs.len())?);
    for (index, requirement) in graph_type.allowed_node_catalogs.iter().enumerate() {
        write_graph_node_catalog_requirement(
            requirement,
            allowed_node_catalogs
                .reborrow()
                .get(capnp_list_index(index)?),
        );
    }

    if let Some(backend) = &graph_type.compiler_backend {
        write_graph_compiler_backend_descriptor(
            backend,
            builder.reborrow().init_compiler_backend(),
        )?;
    }
    if let Some(runtime_product) = &graph_type.runtime_product {
        write_runtime_graph_product_descriptor(
            runtime_product,
            builder.reborrow().init_runtime_product(),
        );
    }
    builder.set_execution_mode(write_graph_execution_mode(graph_type.execution_mode));
    write_graph_palette_policy(
        &graph_type.palette_policy,
        builder.reborrow().init_palette_policy(),
    )?;
    write_text_list(
        &graph_type.tags,
        builder
            .reborrow()
            .init_tags(capnp_list_index(graph_type.tags.len())?),
    );
    Ok(())
}

fn read_graph_type_descriptor(
    reader: project_capnp::graph_type_descriptor::Reader<'_>,
) -> Result<GraphTypeDescriptor, Error> {
    Ok(GraphTypeDescriptor {
        id: GraphTypeId::new(reader.get_id()?.to_string()?),
        version: reader.get_version(),
        display_name: reader.get_display_name()?.to_string()?,
        description: optional_text(reader.has_description(), reader.get_description())?,
        category_path: read_text_list(reader.get_category_path()?)?,
        source_workflow: read_graph_source_workflow(reader.get_source_workflow()?)?,
        template: read_graph_document_template(reader.get_template()?)?,
        allowed_node_catalogs: reader
            .get_allowed_node_catalogs()?
            .iter()
            .map(read_graph_node_catalog_requirement)
            .collect::<Result<Vec<_>, _>>()?,
        compiler_backend: if reader.has_compiler_backend() {
            Some(read_graph_compiler_backend_descriptor(
                reader.get_compiler_backend()?,
            )?)
        } else {
            None
        },
        runtime_product: if reader.has_runtime_product() {
            Some(read_runtime_graph_product_descriptor(
                reader.get_runtime_product()?,
            )?)
        } else {
            None
        },
        execution_mode: read_graph_execution_mode(reader.get_execution_mode()?),
        palette_policy: read_graph_palette_policy(reader.get_palette_policy()?)?,
        tags: read_text_list(reader.get_tags()?)?,
    })
}

fn write_graph_source_workflow(
    workflow: &GraphSourceWorkflow,
    mut builder: project_capnp::graph_source_workflow::Builder<'_>,
) {
    builder.set_workflow_id(&workflow.workflow_id);
    builder.set_kind(match workflow.kind {
        GraphSourceWorkflowKind::ProjectDocument => {
            project_capnp::GraphSourceWorkflowKind::ProjectDocument
        }
        GraphSourceWorkflowKind::File => project_capnp::GraphSourceWorkflowKind::File,
    });
    if let Some(source_schema) = &workflow.source_schema {
        builder.set_source_schema(source_schema);
    }
    if let Some(source_root) = &workflow.source_root {
        builder.set_source_root(source_root);
    }
    if let Some(path_prefix) = &workflow.default_path_prefix {
        builder.set_default_path_prefix(path_prefix);
    }
    if let Some(extension) = &workflow.default_extension {
        builder.set_default_extension(extension);
    }
}

fn read_graph_source_workflow(
    reader: project_capnp::graph_source_workflow::Reader<'_>,
) -> Result<GraphSourceWorkflow, Error> {
    Ok(GraphSourceWorkflow {
        workflow_id: reader.get_workflow_id()?.to_string()?,
        kind: match reader.get_kind()? {
            project_capnp::GraphSourceWorkflowKind::ProjectDocument => {
                GraphSourceWorkflowKind::ProjectDocument
            }
            project_capnp::GraphSourceWorkflowKind::File => GraphSourceWorkflowKind::File,
        },
        source_schema: optional_text(reader.has_source_schema(), reader.get_source_schema())?,
        source_root: optional_text(reader.has_source_root(), reader.get_source_root())?,
        default_path_prefix: optional_text(
            reader.has_default_path_prefix(),
            reader.get_default_path_prefix(),
        )?,
        default_extension: optional_text(
            reader.has_default_extension(),
            reader.get_default_extension(),
        )?,
    })
}

fn write_graph_document_template(
    template: &GraphDocumentTemplate,
    mut builder: project_capnp::graph_document_template::Builder<'_>,
) -> Result<(), Error> {
    write_visual_graph_document(&template.document, builder.reborrow().init_graph())
}

fn read_graph_document_template(
    reader: project_capnp::graph_document_template::Reader<'_>,
) -> Result<GraphDocumentTemplate, Error> {
    Ok(GraphDocumentTemplate {
        document: read_visual_graph_document(reader.get_graph()?)?,
    })
}

fn write_graph_node_catalog_requirement(
    requirement: &GraphNodeCatalogRequirement,
    mut builder: project_capnp::graph_node_catalog_requirement::Builder<'_>,
) {
    builder.set_catalog_id(&requirement.catalog_id);
    if let Some(version) = requirement.minimum_version {
        builder.set_minimum_version(version);
    }
    if let Some(hash) = &requirement.required_hash {
        builder.set_required_hash(hash);
    }
}

fn read_graph_node_catalog_requirement(
    reader: project_capnp::graph_node_catalog_requirement::Reader<'_>,
) -> Result<GraphNodeCatalogRequirement, Error> {
    Ok(GraphNodeCatalogRequirement {
        catalog_id: reader.get_catalog_id()?.to_string()?,
        minimum_version: non_zero_u32_option(reader.get_minimum_version()),
        required_hash: if reader.has_required_hash() {
            Some(reader.get_required_hash()?.to_vec())
        } else {
            None
        },
    })
}

fn write_graph_compiler_backend_descriptor(
    backend: &GraphCompilerBackendDescriptor,
    mut builder: project_capnp::graph_compiler_backend_descriptor::Builder<'_>,
) -> Result<(), Error> {
    builder.set_id(&backend.id);
    write_graph_compiler_backend_kind(&backend.kind, builder.reborrow().init_kind());
    write_text_list(
        &backend.capability_markers,
        builder.init_capability_markers(capnp_list_index(backend.capability_markers.len())?),
    );
    Ok(())
}

fn read_graph_compiler_backend_descriptor(
    reader: project_capnp::graph_compiler_backend_descriptor::Reader<'_>,
) -> Result<GraphCompilerBackendDescriptor, Error> {
    Ok(GraphCompilerBackendDescriptor {
        id: reader.get_id()?.to_string()?,
        kind: read_graph_compiler_backend_kind(reader.get_kind()?)?,
        capability_markers: read_text_list(reader.get_capability_markers()?)?,
    })
}

fn write_graph_compiler_backend_kind(
    kind: &GraphCompilerBackendKind,
    builder: project_capnp::graph_compiler_backend_kind::Builder<'_>,
) {
    match kind {
        GraphCompilerBackendKind::GeneratedRust {
            package,
            entry_symbol,
            abi,
        } => {
            let mut generated = builder.init_generated_rust();
            generated.set_package(package);
            generated.set_entry_symbol(entry_symbol);
            generated.set_abi(write_generated_rust_graph_abi(*abi));
        }
        GraphCompilerBackendKind::PackedIr { ir_schema } => {
            builder.init_packed_ir().set_ir_schema(ir_schema);
        }
        GraphCompilerBackendKind::ShaderPipeline { pipeline_kind } => {
            builder
                .init_shader_pipeline()
                .set_pipeline_kind(pipeline_kind);
        }
        GraphCompilerBackendKind::External { kind, locator } => {
            let mut external = builder.init_external();
            external.set_kind(kind);
            external.set_locator(locator);
        }
    }
}

fn read_graph_compiler_backend_kind(
    reader: project_capnp::graph_compiler_backend_kind::Reader<'_>,
) -> Result<GraphCompilerBackendKind, Error> {
    match reader.which()? {
        project_capnp::graph_compiler_backend_kind::Which::GeneratedRust(value) => {
            let value = value?;
            Ok(GraphCompilerBackendKind::GeneratedRust {
                package: value.get_package()?.to_string()?,
                entry_symbol: value.get_entry_symbol()?.to_string()?,
                abi: read_generated_rust_graph_abi(value.get_abi()?),
            })
        }
        project_capnp::graph_compiler_backend_kind::Which::PackedIr(value) => {
            let value = value?;
            Ok(GraphCompilerBackendKind::PackedIr {
                ir_schema: value.get_ir_schema()?.to_string()?,
            })
        }
        project_capnp::graph_compiler_backend_kind::Which::ShaderPipeline(value) => {
            let value = value?;
            Ok(GraphCompilerBackendKind::ShaderPipeline {
                pipeline_kind: value.get_pipeline_kind()?.to_string()?,
            })
        }
        project_capnp::graph_compiler_backend_kind::Which::External(value) => {
            let value = value?;
            Ok(GraphCompilerBackendKind::External {
                kind: value.get_kind()?.to_string()?,
                locator: value.get_locator()?.to_string()?,
            })
        }
    }
}

const fn write_generated_rust_graph_abi(
    abi: GeneratedRustGraphAbi,
) -> project_capnp::GeneratedRustGraphAbi {
    match abi {
        GeneratedRustGraphAbi::ContextSchedule => {
            project_capnp::GeneratedRustGraphAbi::ContextSchedule
        }
        GeneratedRustGraphAbi::TypedDataflow => project_capnp::GeneratedRustGraphAbi::TypedDataflow,
    }
}

const fn read_generated_rust_graph_abi(
    abi: project_capnp::GeneratedRustGraphAbi,
) -> GeneratedRustGraphAbi {
    match abi {
        project_capnp::GeneratedRustGraphAbi::ContextSchedule => {
            GeneratedRustGraphAbi::ContextSchedule
        }
        project_capnp::GeneratedRustGraphAbi::TypedDataflow => GeneratedRustGraphAbi::TypedDataflow,
    }
}

fn write_runtime_graph_product_descriptor(
    product: &RuntimeGraphProductDescriptor,
    mut builder: project_capnp::runtime_graph_product_descriptor::Builder<'_>,
) {
    builder.set_asset_type(&product.asset_type);
    builder.set_product_kind(&product.product_kind);
    builder.set_streamable(product.streamable);
    builder.set_diffable_chunks(product.diffable_chunks);
    write_runtime_graph_execution_strategy(
        &product.execution_strategy,
        builder.reborrow().init_execution_strategy(),
    );
}

fn read_runtime_graph_product_descriptor(
    reader: project_capnp::runtime_graph_product_descriptor::Reader<'_>,
) -> Result<RuntimeGraphProductDescriptor, Error> {
    Ok(RuntimeGraphProductDescriptor {
        asset_type: reader.get_asset_type()?.to_string()?,
        product_kind: reader.get_product_kind()?.to_string()?,
        streamable: reader.get_streamable(),
        diffable_chunks: reader.get_diffable_chunks(),
        execution_strategy: if reader.has_execution_strategy() {
            read_runtime_graph_execution_strategy(reader.get_execution_strategy()?)?
        } else {
            return Err(Error::failed(
                "runtime graph product descriptor is missing execution strategy".to_string(),
            ));
        },
    })
}

fn write_runtime_graph_execution_strategy(
    strategy: &RuntimeGraphExecutionStrategy,
    mut builder: project_capnp::runtime_graph_execution_strategy::Builder<'_>,
) {
    match strategy {
        RuntimeGraphExecutionStrategy::PackedIr => {
            builder.set_packed_ir(());
        }
        RuntimeGraphExecutionStrategy::AotCompiledCode {
            language,
            package,
            entry_symbol,
            context_type,
        } => {
            let mut aot = builder.init_aot_compiled_code();
            aot.set_language(language);
            aot.set_package(package);
            aot.set_entry_symbol(entry_symbol);
            aot.set_context_type(context_type);
        }
        RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { abi, entry_symbol } => {
            let mut module = builder.init_hot_reloaded_compiled_module();
            module.set_abi(abi);
            module.set_entry_symbol(entry_symbol);
        }
        RuntimeGraphExecutionStrategy::ShaderPipeline { pipeline_kind } => {
            builder
                .init_shader_pipeline()
                .set_pipeline_kind(pipeline_kind);
        }
        RuntimeGraphExecutionStrategy::External { kind, locator } => {
            let mut external = builder.init_external();
            external.set_kind(kind);
            external.set_locator(locator);
        }
    }
}

fn read_runtime_graph_execution_strategy(
    reader: project_capnp::runtime_graph_execution_strategy::Reader<'_>,
) -> Result<RuntimeGraphExecutionStrategy, Error> {
    match reader.which()? {
        project_capnp::runtime_graph_execution_strategy::Which::PackedIr(()) => {
            Ok(RuntimeGraphExecutionStrategy::PackedIr)
        }
        project_capnp::runtime_graph_execution_strategy::Which::AotCompiledCode(value) => {
            let value = value?;
            Ok(RuntimeGraphExecutionStrategy::AotCompiledCode {
                language: value.get_language()?.to_string()?,
                package: value.get_package()?.to_string()?,
                entry_symbol: value.get_entry_symbol()?.to_string()?,
                context_type: value.get_context_type()?.to_string()?,
            })
        }
        project_capnp::runtime_graph_execution_strategy::Which::HotReloadedCompiledModule(
            value,
        ) => {
            let value = value?;
            Ok(RuntimeGraphExecutionStrategy::HotReloadedCompiledModule {
                abi: value.get_abi()?.to_string()?,
                entry_symbol: value.get_entry_symbol()?.to_string()?,
            })
        }
        project_capnp::runtime_graph_execution_strategy::Which::ShaderPipeline(value) => {
            let value = value?;
            Ok(RuntimeGraphExecutionStrategy::ShaderPipeline {
                pipeline_kind: value.get_pipeline_kind()?.to_string()?,
            })
        }
        project_capnp::runtime_graph_execution_strategy::Which::External(value) => {
            let value = value?;
            Ok(RuntimeGraphExecutionStrategy::External {
                kind: value.get_kind()?.to_string()?,
                locator: value.get_locator()?.to_string()?,
            })
        }
    }
}

const fn write_graph_execution_mode(mode: GraphExecutionMode) -> project_capnp::GraphExecutionMode {
    match mode {
        GraphExecutionMode::RuntimeCompiled => project_capnp::GraphExecutionMode::RuntimeCompiled,
        GraphExecutionMode::EditorInterpreted => {
            project_capnp::GraphExecutionMode::EditorInterpreted
        }
        GraphExecutionMode::RuntimeCompiledAndEditorInterpreted => {
            project_capnp::GraphExecutionMode::RuntimeCompiledAndEditorInterpreted
        }
    }
}

const fn read_graph_execution_mode(mode: project_capnp::GraphExecutionMode) -> GraphExecutionMode {
    match mode {
        project_capnp::GraphExecutionMode::RuntimeCompiled => GraphExecutionMode::RuntimeCompiled,
        project_capnp::GraphExecutionMode::EditorInterpreted => {
            GraphExecutionMode::EditorInterpreted
        }
        project_capnp::GraphExecutionMode::RuntimeCompiledAndEditorInterpreted => {
            GraphExecutionMode::RuntimeCompiledAndEditorInterpreted
        }
    }
}

fn write_graph_palette_policy(
    policy: &GraphPalettePolicy,
    mut builder: project_capnp::graph_palette_policy::Builder<'_>,
) -> Result<(), Error> {
    write_text_list(
        &policy.root_categories,
        builder
            .reborrow()
            .init_root_categories(capnp_list_index(policy.root_categories.len())?),
    );
    write_text_list(
        &policy.required_node_capabilities,
        builder
            .reborrow()
            .init_required_node_capabilities(capnp_list_index(
                policy.required_node_capabilities.len(),
            )?),
    );
    write_text_list(
        &policy.hidden_node_tags,
        builder.init_hidden_node_tags(capnp_list_index(policy.hidden_node_tags.len())?),
    );
    Ok(())
}

fn read_graph_palette_policy(
    reader: project_capnp::graph_palette_policy::Reader<'_>,
) -> Result<GraphPalettePolicy, Error> {
    Ok(GraphPalettePolicy {
        root_categories: read_text_list(reader.get_root_categories()?)?,
        required_node_capabilities: read_text_list(reader.get_required_node_capabilities()?)?,
        hidden_node_tags: read_text_list(reader.get_hidden_node_tags()?)?,
    })
}

fn write_graph_command(
    command: &GraphCommand,
    builder: project_capnp::graph_command::Builder<'_>,
) -> Result<(), Error> {
    validate_graph_command_for_transport(command)?;
    match command {
        GraphCommand::AddNode { node } => write_graph_node(node, builder.init_add_node())?,
        GraphCommand::RemoveNode { node_id } => {
            write_graph_node_id(*node_id, builder.init_remove_node());
        }
        GraphCommand::SetInputValue {
            node_id,
            port_id,
            value,
        } => {
            let mut input = builder.init_set_input_value();
            write_graph_node_id(*node_id, input.reborrow().init_node());
            input.set_port(port_id.0);
            if let Some(value) = value {
                value.to_capnp(input.init_value())?;
            } else {
                input.set_clear(());
            }
        }
        GraphCommand::MoveNode { node_id, layout } => {
            let mut move_node = builder.init_move_node();
            write_graph_node_id(*node_id, move_node.reborrow().init_node());
            write_graph_node_layout(*layout, move_node.init_layout());
        }
        GraphCommand::Connect { connection } => {
            write_graph_connection(connection, builder.init_connect())?;
        }
        GraphCommand::SetConnectionRoute {
            connection_id,
            route,
        } => {
            let mut command = builder.init_set_connection_route();
            write_graph_connection_id(*connection_id, command.reborrow().init_connection());
            write_graph_connection_route(route, command.init_route())?;
        }
        GraphCommand::Disconnect { connection_id } => {
            write_graph_connection_id(*connection_id, builder.init_disconnect());
        }
        GraphCommand::UpsertComment { comment } => {
            write_graph_comment(comment, builder.init_upsert_comment());
        }
        GraphCommand::RemoveComment { comment_id } => {
            write_graph_comment_id(*comment_id, builder.init_remove_comment());
        }
    }
    Ok(())
}

fn read_graph_command(
    reader: project_capnp::graph_command::Reader<'_>,
) -> Result<GraphCommand, Error> {
    let command = match reader.which()? {
        project_capnp::graph_command::Which::AddNode(node) => GraphCommand::AddNode {
            node: read_graph_node(node?)?,
        },
        project_capnp::graph_command::Which::RemoveNode(node_id) => GraphCommand::RemoveNode {
            node_id: read_graph_node_id(node_id?)?,
        },
        project_capnp::graph_command::Which::SetInputValue(input) => {
            let input = input?;
            let value = match input.which()? {
                project_capnp::graph_input_value_command::Which::Clear(()) => None,
                project_capnp::graph_input_value_command::Which::Value(value) => {
                    Some(vnext::ReflectedValueEnvelope::from_capnp(value?)?)
                }
            };
            GraphCommand::SetInputValue {
                node_id: read_graph_node_id(input.get_node()?)?,
                port_id: NodePortId(input.get_port()),
                value,
            }
        }
        project_capnp::graph_command::Which::MoveNode(move_node) => {
            let move_node = move_node?;
            GraphCommand::MoveNode {
                node_id: read_graph_node_id(move_node.get_node()?)?,
                layout: read_graph_node_layout(move_node.get_layout()?)?,
            }
        }
        project_capnp::graph_command::Which::Connect(connection) => GraphCommand::Connect {
            connection: read_graph_connection(connection?)?,
        },
        project_capnp::graph_command::Which::SetConnectionRoute(command) => {
            let command = command?;
            GraphCommand::SetConnectionRoute {
                connection_id: read_graph_connection_id(command.get_connection()?)?,
                route: read_graph_connection_route(command.get_route()?)?,
            }
        }
        project_capnp::graph_command::Which::Disconnect(connection_id) => {
            GraphCommand::Disconnect {
                connection_id: read_graph_connection_id(connection_id?)?,
            }
        }
        project_capnp::graph_command::Which::UpsertComment(comment) => {
            GraphCommand::UpsertComment {
                comment: read_graph_comment(comment?)?,
            }
        }
        project_capnp::graph_command::Which::RemoveComment(comment_id) => {
            GraphCommand::RemoveComment {
                comment_id: read_graph_comment_id(comment_id?)?,
            }
        }
    };
    validate_graph_command_for_transport(&command)?;
    Ok(command)
}

fn write_visual_graph_document(
    document: &VisualGraphDocument,
    mut builder: project_capnp::visual_graph_document::Builder<'_>,
) -> Result<(), Error> {
    validate_visual_graph_document_for_transport(document)?;
    builder.set_document_version(document.document_version);
    builder.set_graph_type(&document.graph_type);
    if let Some(hash) = &document.required_catalog_hash {
        builder.reborrow().set_required_catalog_hash(hash);
    } else {
        builder.reborrow().set_no_required_catalog_hash(());
    }

    let mut nodes = builder
        .reborrow()
        .init_nodes(capnp_list_index(document.nodes.len())?);
    for (index, node) in document.nodes.iter().enumerate() {
        write_graph_node(node, nodes.reborrow().get(capnp_list_index(index)?))?;
    }

    let mut connections = builder
        .reborrow()
        .init_connections(capnp_list_index(document.connections.len())?);
    for (index, connection) in document.connections.iter().enumerate() {
        write_graph_connection(
            connection,
            connections.reborrow().get(capnp_list_index(index)?),
        )?;
    }

    let mut comments = builder
        .reborrow()
        .init_comments(capnp_list_index(document.comments.len())?);
    for (index, comment) in document.comments.iter().enumerate() {
        write_graph_comment(comment, comments.reborrow().get(capnp_list_index(index)?));
    }
    Ok(())
}

fn read_visual_graph_document(
    reader: project_capnp::visual_graph_document::Reader<'_>,
) -> Result<VisualGraphDocument, Error> {
    let required_catalog_hash = match reader.which()? {
        project_capnp::visual_graph_document::Which::NoRequiredCatalogHash(()) => None,
        project_capnp::visual_graph_document::Which::RequiredCatalogHash(hash) => {
            Some(hash?.to_vec())
        }
    };
    let document = VisualGraphDocument {
        document_version: reader.get_document_version(),
        graph_type: reader.get_graph_type()?.to_string()?,
        required_catalog_hash,
        nodes: reader
            .get_nodes()?
            .iter()
            .map(read_graph_node)
            .collect::<Result<Vec<_>, _>>()?,
        connections: reader
            .get_connections()?
            .iter()
            .map(read_graph_connection)
            .collect::<Result<Vec<_>, _>>()?,
        comments: reader
            .get_comments()?
            .iter()
            .map(read_graph_comment)
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_visual_graph_document_for_transport(&document)?;
    Ok(document)
}

fn write_graph_node(
    node: &GraphNode,
    mut builder: project_capnp::graph_node::Builder<'_>,
) -> Result<(), Error> {
    write_graph_node_id(node.id, builder.reborrow().init_id());
    builder.set_node_type(node.node_type.as_str());
    builder.set_node_type_version(node.node_type_version);
    let mut input_values = builder
        .reborrow()
        .init_input_values(capnp_list_index(node.input_values.len())?);
    for (index, (port_id, value)) in node.input_values.iter().enumerate() {
        let mut input = input_values.reborrow().get(capnp_list_index(index)?);
        input.set_port(port_id.0);
        value.to_capnp(input.init_value())?;
    }
    write_graph_node_layout(node.layout, builder.init_layout());
    Ok(())
}

fn read_graph_node(reader: project_capnp::graph_node::Reader<'_>) -> Result<GraphNode, Error> {
    let mut seen_ports = BTreeSet::new();
    let mut input_values = BTreeMap::new();
    for input in reader.get_input_values()? {
        let port_id = NodePortId(input.get_port());
        validate_graph_port_id_for_transport("graph node input value", port_id)?;
        if !seen_ports.insert(port_id) {
            return Err(invalid_project_protocol_value(
                "graph node input value",
                format!("duplicate input port id {}", port_id.0),
            ));
        }
        input_values.insert(
            port_id,
            vnext::ReflectedValueEnvelope::from_capnp(input.get_value()?)?,
        );
    }

    let node = GraphNode {
        id: read_graph_node_id(reader.get_id()?)?,
        node_type: NodeTypeId::new(reader.get_node_type()?.to_string()?),
        node_type_version: reader.get_node_type_version(),
        input_values,
        layout: read_graph_node_layout(reader.get_layout()?)?,
    };
    validate_graph_node_for_transport(&node)?;
    Ok(node)
}

fn write_graph_connection(
    connection: &GraphConnection,
    mut builder: project_capnp::graph_connection::Builder<'_>,
) -> Result<(), Error> {
    write_graph_connection_id(connection.id, builder.reborrow().init_id());
    write_graph_port_ref(&connection.from, builder.reborrow().init_from());
    write_graph_port_ref(&connection.to, builder.reborrow().init_to());
    write_graph_connection_route(&connection.route, builder.init_route())?;
    Ok(())
}

fn read_graph_connection(
    reader: project_capnp::graph_connection::Reader<'_>,
) -> Result<GraphConnection, Error> {
    let connection = GraphConnection {
        id: read_graph_connection_id(reader.get_id()?)?,
        from: read_graph_port_ref(reader.get_from()?)?,
        to: read_graph_port_ref(reader.get_to()?)?,
        route: if reader.has_route() {
            read_graph_connection_route(reader.get_route()?)?
        } else {
            GraphConnectionRoute::default()
        },
    };
    validate_graph_connection_for_transport(&connection)?;
    Ok(connection)
}

fn write_graph_connection_route(
    route: &GraphConnectionRoute,
    mut builder: project_capnp::graph_connection_route::Builder<'_>,
) -> Result<(), Error> {
    builder.set_style(match route.style {
        GraphRouteStyle::Orthogonal => project_capnp::GraphRouteStyle::Orthogonal,
        GraphRouteStyle::Polyline => project_capnp::GraphRouteStyle::Polyline,
        GraphRouteStyle::Spline => project_capnp::GraphRouteStyle::Spline,
    });
    let mut anchors = builder
        .reborrow()
        .init_anchors(capnp_list_index(route.anchors.len())?);
    for (index, anchor) in route.anchors.iter().enumerate() {
        write_graph_route_anchor(anchor, anchors.reborrow().get(capnp_list_index(index)?));
    }
    Ok(())
}

fn read_graph_connection_route(
    reader: project_capnp::graph_connection_route::Reader<'_>,
) -> Result<GraphConnectionRoute, Error> {
    let route = GraphConnectionRoute {
        style: match reader.get_style()? {
            project_capnp::GraphRouteStyle::Orthogonal => GraphRouteStyle::Orthogonal,
            project_capnp::GraphRouteStyle::Polyline => GraphRouteStyle::Polyline,
            project_capnp::GraphRouteStyle::Spline => GraphRouteStyle::Spline,
        },
        anchors: reader
            .get_anchors()?
            .iter()
            .map(read_graph_route_anchor)
            .collect::<Result<Vec<_>, _>>()?,
    };
    validate_graph_connection_route_for_transport("graph connection route", &route)?;
    Ok(route)
}

fn write_graph_route_anchor(
    anchor: &GraphRouteAnchor,
    mut builder: project_capnp::graph_route_anchor::Builder<'_>,
) {
    write_graph_route_anchor_id(anchor.id, builder.reborrow().init_id());
    write_graph_point(anchor.position, builder.reborrow().init_position());
    builder.set_kind(match anchor.kind {
        GraphRouteAnchorKind::UserWaypoint => project_capnp::GraphRouteAnchorKind::UserWaypoint,
        GraphRouteAnchorKind::SolverWaypoint => project_capnp::GraphRouteAnchorKind::SolverWaypoint,
        GraphRouteAnchorKind::Junction => project_capnp::GraphRouteAnchorKind::Junction,
    });
    builder.set_outgoing_segment(match anchor.outgoing_segment {
        GraphRouteSegmentConstraint::Flexible => {
            project_capnp::GraphRouteSegmentConstraint::Flexible
        }
        GraphRouteSegmentConstraint::Fixed => project_capnp::GraphRouteSegmentConstraint::Fixed,
    });
}

fn read_graph_route_anchor(
    reader: project_capnp::graph_route_anchor::Reader<'_>,
) -> Result<GraphRouteAnchor, Error> {
    Ok(GraphRouteAnchor {
        id: read_graph_route_anchor_id(reader.get_id()?)?,
        position: read_graph_point(reader.get_position()?)?,
        kind: match reader.get_kind()? {
            project_capnp::GraphRouteAnchorKind::UserWaypoint => GraphRouteAnchorKind::UserWaypoint,
            project_capnp::GraphRouteAnchorKind::SolverWaypoint => {
                GraphRouteAnchorKind::SolverWaypoint
            }
            project_capnp::GraphRouteAnchorKind::Junction => GraphRouteAnchorKind::Junction,
        },
        outgoing_segment: match reader.get_outgoing_segment()? {
            project_capnp::GraphRouteSegmentConstraint::Flexible => {
                GraphRouteSegmentConstraint::Flexible
            }
            project_capnp::GraphRouteSegmentConstraint::Fixed => GraphRouteSegmentConstraint::Fixed,
        },
    })
}

fn write_graph_port_ref(
    port: &GraphPortRef,
    mut builder: project_capnp::graph_port_ref::Builder<'_>,
) {
    write_graph_node_id(port.node_id, builder.reborrow().init_node());
    builder.set_port(port.port_id.0);
}

fn read_graph_port_ref(
    reader: project_capnp::graph_port_ref::Reader<'_>,
) -> Result<GraphPortRef, Error> {
    let port = GraphPortRef {
        node_id: read_graph_node_id(reader.get_node()?)?,
        port_id: NodePortId(reader.get_port()),
    };
    validate_graph_port_ref_for_transport("graph port ref", &port)?;
    Ok(port)
}

fn write_graph_comment(
    comment: &GraphComment,
    mut builder: project_capnp::graph_comment::Builder<'_>,
) {
    write_graph_comment_id(comment.id, builder.reborrow().init_id());
    builder.set_text(&comment.text);
    write_graph_comment_bounds(comment.bounds, builder.init_bounds());
}

fn read_graph_comment(
    reader: project_capnp::graph_comment::Reader<'_>,
) -> Result<GraphComment, Error> {
    let comment = GraphComment {
        id: read_graph_comment_id(reader.get_id()?)?,
        text: reader.get_text()?.to_string()?,
        bounds: read_graph_comment_bounds(reader.get_bounds()?)?,
    };
    validate_graph_comment_for_transport(&comment)?;
    Ok(comment)
}

fn write_graph_node_layout(
    layout: GraphNodeLayout,
    mut builder: project_capnp::graph_node_layout::Builder<'_>,
) {
    builder.set_x(layout.x);
    builder.set_y(layout.y);
}

fn read_graph_node_layout(
    reader: project_capnp::graph_node_layout::Reader<'_>,
) -> Result<GraphNodeLayout, Error> {
    let layout = GraphNodeLayout {
        x: reader.get_x(),
        y: reader.get_y(),
    };
    validate_graph_node_layout_for_transport("graph node layout", layout)?;
    Ok(layout)
}

fn write_graph_point(point: GraphPoint, mut builder: project_capnp::graph_point::Builder<'_>) {
    builder.set_x(point.x);
    builder.set_y(point.y);
}

fn read_graph_point(reader: project_capnp::graph_point::Reader<'_>) -> Result<GraphPoint, Error> {
    let point = GraphPoint {
        x: reader.get_x(),
        y: reader.get_y(),
    };
    validate_graph_point_for_transport("graph point", "point", point)?;
    Ok(point)
}

fn write_graph_comment_bounds(
    bounds: GraphCommentBounds,
    mut builder: project_capnp::graph_comment_bounds::Builder<'_>,
) {
    builder.set_x(bounds.x);
    builder.set_y(bounds.y);
    builder.set_width(bounds.width);
    builder.set_height(bounds.height);
}

fn read_graph_comment_bounds(
    reader: project_capnp::graph_comment_bounds::Reader<'_>,
) -> Result<GraphCommentBounds, Error> {
    let bounds = GraphCommentBounds {
        x: reader.get_x(),
        y: reader.get_y(),
        width: reader.get_width(),
        height: reader.get_height(),
    };
    validate_graph_comment_bounds_for_transport("graph comment bounds", bounds)?;
    Ok(bounds)
}

fn write_graph_node_id(id: GraphNodeId, mut builder: project_capnp::graph_node_id::Builder<'_>) {
    builder.set_uuid(id.as_uuid().as_bytes());
}

fn read_graph_node_id(
    reader: project_capnp::graph_node_id::Reader<'_>,
) -> Result<GraphNodeId, Error> {
    Ok(GraphNodeId::new(read_uuid_data(reader.get_uuid()?)?))
}

fn write_graph_connection_id(
    id: GraphConnectionId,
    mut builder: project_capnp::graph_connection_id::Builder<'_>,
) {
    builder.set_uuid(id.as_uuid().as_bytes());
}

fn read_graph_connection_id(
    reader: project_capnp::graph_connection_id::Reader<'_>,
) -> Result<GraphConnectionId, Error> {
    Ok(GraphConnectionId::new(read_uuid_data(reader.get_uuid()?)?))
}

fn write_graph_route_anchor_id(
    id: GraphRouteAnchorId,
    mut builder: project_capnp::graph_route_anchor_id::Builder<'_>,
) {
    builder.set_uuid(id.as_uuid().as_bytes());
}

fn read_graph_route_anchor_id(
    reader: project_capnp::graph_route_anchor_id::Reader<'_>,
) -> Result<GraphRouteAnchorId, Error> {
    Ok(GraphRouteAnchorId::new(read_uuid_data(reader.get_uuid()?)?))
}

fn write_graph_comment_id(
    id: GraphCommentId,
    mut builder: project_capnp::graph_comment_id::Builder<'_>,
) {
    builder.set_uuid(id.as_uuid().as_bytes());
}

fn read_graph_comment_id(
    reader: project_capnp::graph_comment_id::Reader<'_>,
) -> Result<GraphCommentId, Error> {
    Ok(GraphCommentId::new(read_uuid_data(reader.get_uuid()?)?))
}

fn write_graph_command_diagnostic(
    diagnostic: &GraphCommandDiagnostic,
    mut builder: project_capnp::graph_command_diagnostic::Builder<'_>,
) {
    builder.set_message(&diagnostic.message);
    builder.set_severity(match diagnostic.severity {
        GraphDiagnosticSeverity::Info => project_capnp::GraphDiagnosticSeverity::Info,
        GraphDiagnosticSeverity::Warning => project_capnp::GraphDiagnosticSeverity::Warning,
        GraphDiagnosticSeverity::Error => project_capnp::GraphDiagnosticSeverity::Error,
    });
    if let Some(command_index) = diagnostic.command_index {
        builder.set_command_index(command_index);
    } else {
        builder.set_no_command_index(());
    }
}

fn read_graph_command_diagnostic(
    reader: project_capnp::graph_command_diagnostic::Reader<'_>,
) -> Result<GraphCommandDiagnostic, Error> {
    let command_index = match reader.which()? {
        project_capnp::graph_command_diagnostic::Which::NoCommandIndex(()) => None,
        project_capnp::graph_command_diagnostic::Which::CommandIndex(value) => Some(value),
    };
    let diagnostic = GraphCommandDiagnostic {
        command_index,
        severity: match reader.get_severity()? {
            project_capnp::GraphDiagnosticSeverity::Info => GraphDiagnosticSeverity::Info,
            project_capnp::GraphDiagnosticSeverity::Warning => GraphDiagnosticSeverity::Warning,
            project_capnp::GraphDiagnosticSeverity::Error => GraphDiagnosticSeverity::Error,
        },
        message: reader.get_message()?.to_string()?,
    };
    validate_graph_command_diagnostic_for_transport(&diagnostic)?;
    Ok(diagnostic)
}

fn write_project_inventory_report(
    report: &ProjectInventoryReport,
    mut builder: project_capnp::project_inventory_report::Builder<'_>,
) -> Result<(), Error> {
    builder.set_service_role(&report.service_role);
    write_project_inventory_lock_status(&report.lock_status, builder.reborrow().init_lock_status());
    let mut gems = builder
        .reborrow()
        .init_gems(capnp_list_index(report.gems.len())?);
    for (index, gem) in report.gems.iter().enumerate() {
        write_project_inventory_gem(gem, gems.reborrow().get(capnp_list_index(index)?))?;
    }
    write_project_inventory_registry_counts(&report.registry, builder.reborrow().init_registry());
    write_text_list(
        &report.diagnostics,
        builder
            .reborrow()
            .init_diagnostics(capnp_list_index(report.diagnostics.len())?),
    );
    builder.set_degraded(report.degraded);
    Ok(())
}

fn read_project_inventory_report(
    reader: project_capnp::project_inventory_report::Reader<'_>,
) -> Result<ProjectInventoryReport, Error> {
    Ok(ProjectInventoryReport {
        service_role: reader.get_service_role()?.to_string()?,
        lock_status: read_project_inventory_lock_status(reader.get_lock_status()?)?,
        gems: reader
            .get_gems()?
            .iter()
            .map(read_project_inventory_gem)
            .collect::<Result<Vec<_>, _>>()?,
        registry: read_project_inventory_registry_counts(reader.get_registry()?),
        diagnostics: read_text_list(reader.get_diagnostics()?)?,
        degraded: reader.get_degraded(),
    })
}

fn write_project_inventory_lock_status(
    status: &ProjectInventoryLockStatus,
    mut builder: project_capnp::project_inventory_lock_status::Builder<'_>,
) {
    builder.set_state(write_project_inventory_lock_state(status.state));
    builder.set_path(&status.path);
    builder.set_diagnostic(&status.diagnostic);
}

fn read_project_inventory_lock_status(
    reader: project_capnp::project_inventory_lock_status::Reader<'_>,
) -> Result<ProjectInventoryLockStatus, Error> {
    Ok(ProjectInventoryLockStatus {
        state: read_project_inventory_lock_state(reader.get_state()?),
        path: reader.get_path()?.to_string()?,
        diagnostic: reader.get_diagnostic()?.to_string()?,
    })
}

fn write_project_inventory_registry_counts(
    counts: &ProjectInventoryRegistryCounts,
    mut builder: project_capnp::project_inventory_registry_counts::Builder<'_>,
) {
    builder.set_build_rules(counts.build_rules);
    builder.set_node_types(counts.node_types);
    builder.set_graph_types(counts.graph_types);
}

fn read_project_inventory_registry_counts(
    reader: project_capnp::project_inventory_registry_counts::Reader<'_>,
) -> ProjectInventoryRegistryCounts {
    ProjectInventoryRegistryCounts {
        build_rules: reader.get_build_rules(),
        node_types: reader.get_node_types(),
        graph_types: reader.get_graph_types(),
    }
}

fn write_project_inventory_gem(
    gem: &ProjectInventoryGem,
    mut builder: project_capnp::project_inventory_gem::Builder<'_>,
) -> Result<(), Error> {
    builder.set_id(&gem.id);
    builder.set_expected_package(&gem.expected_package);
    builder.set_name(&gem.name);
    builder.set_version(&gem.version);
    builder.set_kind(write_project_inventory_gem_kind(gem.kind));
    builder.set_expected(gem.expected);
    builder.set_active(gem.active);
    write_text_list(
        &gem.capabilities,
        builder
            .reborrow()
            .init_capabilities(capnp_list_index(gem.capabilities.len())?),
    );
    Ok(())
}

fn read_project_inventory_gem(
    reader: project_capnp::project_inventory_gem::Reader<'_>,
) -> Result<ProjectInventoryGem, Error> {
    Ok(ProjectInventoryGem {
        id: reader.get_id()?.to_string()?,
        expected_package: reader.get_expected_package()?.to_string()?,
        name: reader.get_name()?.to_string()?,
        version: reader.get_version()?.to_string()?,
        kind: read_project_inventory_gem_kind(reader.get_kind()?),
        expected: reader.get_expected(),
        active: reader.get_active(),
        capabilities: read_text_list(reader.get_capabilities()?)?,
    })
}

const fn write_project_inventory_gem_kind(
    kind: ProjectInventoryGemKind,
) -> project_capnp::ProjectInventoryGemKind {
    match kind {
        ProjectInventoryGemKind::Project => project_capnp::ProjectInventoryGemKind::Project,
        ProjectInventoryGemKind::Engine => project_capnp::ProjectInventoryGemKind::Engine,
        ProjectInventoryGemKind::Unknown => project_capnp::ProjectInventoryGemKind::Unknown,
    }
}

const fn read_project_inventory_gem_kind(
    kind: project_capnp::ProjectInventoryGemKind,
) -> ProjectInventoryGemKind {
    match kind {
        project_capnp::ProjectInventoryGemKind::Project => ProjectInventoryGemKind::Project,
        project_capnp::ProjectInventoryGemKind::Engine => ProjectInventoryGemKind::Engine,
        project_capnp::ProjectInventoryGemKind::Unknown => ProjectInventoryGemKind::Unknown,
    }
}

const fn write_project_inventory_lock_state(
    state: ProjectInventoryLockState,
) -> project_capnp::ProjectInventoryLockState {
    match state {
        ProjectInventoryLockState::Fresh => project_capnp::ProjectInventoryLockState::Fresh,
        ProjectInventoryLockState::Missing => project_capnp::ProjectInventoryLockState::Missing,
        ProjectInventoryLockState::Stale => project_capnp::ProjectInventoryLockState::Stale,
        ProjectInventoryLockState::Unavailable => {
            project_capnp::ProjectInventoryLockState::Unavailable
        }
    }
}

const fn read_project_inventory_lock_state(
    state: project_capnp::ProjectInventoryLockState,
) -> ProjectInventoryLockState {
    match state {
        project_capnp::ProjectInventoryLockState::Fresh => ProjectInventoryLockState::Fresh,
        project_capnp::ProjectInventoryLockState::Missing => ProjectInventoryLockState::Missing,
        project_capnp::ProjectInventoryLockState::Stale => ProjectInventoryLockState::Stale,
        project_capnp::ProjectInventoryLockState::Unavailable => {
            ProjectInventoryLockState::Unavailable
        }
    }
}

fn write_text_list(values: &[String], mut builder: capnp::text_list::Builder<'_>) {
    for (index, value) in values.iter().enumerate() {
        builder.set(capnp_bounded_index(index), value);
    }
}

fn read_text_list(reader: capnp::text_list::Reader<'_>) -> Result<Vec<String>, Error> {
    reader
        .iter()
        .map(|value| value.and_then(|value| Ok(value.to_string()?)))
        .collect()
}

const fn non_zero_u32_option(value: u32) -> Option<u32> {
    if value == 0 { None } else { Some(value) }
}

fn optional_text(
    has_value: bool,
    value: capnp::Result<capnp::text::Reader<'_>>,
) -> Result<Option<String>, Error> {
    if has_value {
        Ok(Some(value?.to_string()?))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use capnp::{message, serialize_packed};

    use super::*;

    fn test_session_id() -> Uuid {
        Uuid::from_bytes([0x66; 16])
    }

    fn project_request_capability(permission: &'static str) -> Capability {
        core::Capability::new(
            core::ServiceId::new(core::EDITOR_SERVICE_NAMESPACE, core::EDITOR_SERVICE_NAME),
            core::ServiceRole::Editor,
        )
        .with_audience(PROJECT_HOST_AUDIENCE)
        .with_session(test_session_id())
        .with_permissions([permission])
    }
    /// The single manager entry inside [`gamedata_catalog_snapshot_fixture`].
    fn gamedata_manager_entry_fixture() -> GameDataManagerCatalogEntry {
        GameDataManagerCatalogEntry {
            key: "manager:ItemManager".to_string(),
            name: "ItemManager".to_string(),
            owner: "test-gem".to_string(),
            row_type: "ItemRow".to_string(),
            kind: "CRC key projection".to_string(),
            output_type: "ItemRow".to_string(),
            read_only: false,
            provider_target: Some(GameDataProviderTarget {
                kind: "table".to_string(),
                name: "Items".to_string(),
                row_type: "ItemRow".to_string(),
            }),
            key_policy: GameDataKeyPolicy {
                kind: "CRC32".to_string(),
                transforms: vec!["trim".to_string(), "lowercase".to_string()],
                reject_zero_crc: true,
                store_key_text: true,
            },
            duplicate_key_policy: "overwrite".to_string(),
            inputs: vec![GameDataManagerInput {
                kind: "table".to_string(),
                name: "Items".to_string(),
                row_type: "ItemRow".to_string(),
                source_root: "gamedata".to_string(),
                source_path: "items.ron".to_string(),
                detail: "gamedata:items.ron".to_string(),
                provider_kind: "table".to_string(),
            }],
            row_filters: vec![GameDataRowFilter {
                field: "disabled".to_string(),
                predicate: "field false".to_string(),
                compare_field: String::new(),
            }],
            projection_transforms: vec![GameDataProjectionTransform {
                field: "item_id_crc".to_string(),
                source_column: "ItemID".to_string(),
                kind: "lowercase CRC string".to_string(),
            }],
            secondary_indexes: vec![GameDataSecondaryIndex {
                name: "by_tier".to_string(),
                field: "tier".to_string(),
                key_kind: "u16".to_string(),
                storage: "sparse vec".to_string(),
                duplicate_key_policy: "multi".to_string(),
            }],
            source_targets: vec![GameDataProviderTarget {
                kind: "table".to_string(),
                name: "Items".to_string(),
                row_type: "ItemRow".to_string(),
            }],
            dependencies: vec![GameDataManagerNodeRef {
                key: "provider:table:Items".to_string(),
                label: "Items".to_string(),
                kind: "provider".to_string(),
            }],
            dependents: vec![GameDataManagerNodeRef {
                key: "manager:LootManager".to_string(),
                label: "LootManager".to_string(),
                kind: "authored".to_string(),
            }],
            diagnostics: vec![GameDataCatalogDiagnostic {
                code: "missing-schema-hash".to_string(),
                message: "table provider `Items` is missing a row schema hash".to_string(),
                target_key: "manager:ItemManager".to_string(),
                target_label: "ItemManager".to_string(),
            }],
            projection_hash: vec![3; 32],
        }
    }

    /// The catalog fixture the round-trip test encodes and decodes.
    fn gamedata_catalog_snapshot_fixture() -> GameDataCatalogSnapshot {
        GameDataCatalogSnapshot::new(
            GAMEDATA_CATALOG_SNAPSHOT_VERSION,
            123,
            vec![GameDataTableDescriptor {
                name: "Items".to_string(),
                row_type: "ItemRow".to_string(),
                source_root: "gamedata".to_string(),
                source_path: "items.ron".to_string(),
                owner: "test-gem".to_string(),
                schema_hash: Some(99),
                document_id: "gamedata/items.ron".to_string(),
                schema_type: "ItemRow".to_string(),
                category: "items".to_string(),
                row_count: Some(12),
                families: vec!["ItemFamily".to_string()],
                source_ref: WorkspaceSourceFileRef {
                    source_root_key: "project:test:assets".to_string(),
                    source_path: "items.ron".to_string(),
                    schema_type: "azoth.gamedata.TableSource".to_string(),
                },
            }],
            vec![GameDataTableFamilyDescriptor {
                name: "ItemFamily".to_string(),
                row_type: "ItemRow".to_string(),
                owner: "test-gem".to_string(),
                duplicate_key_policy: "overwrite".to_string(),
                tables: vec!["Items".to_string()],
            }],
            vec![gamedata_manager_entry_fixture()],
            vec![GameDataCatalogDiagnostic {
                code: "missing-schema-hash".to_string(),
                message: "table provider `Items` is missing a row schema hash".to_string(),
                target_key: "manager:ItemManager".to_string(),
                target_label: "ItemManager".to_string(),
            }],
        )
    }

    #[test]
    fn gamedata_catalog_snapshot_round_trips() {
        let snapshot = gamedata_catalog_snapshot_fixture();

        let bytes = encode_gamedata_catalog_snapshot(&snapshot).unwrap();
        let decoded = decode_gamedata_catalog_snapshot_packed(&bytes).unwrap();

        assert_eq!(decoded, snapshot);
    }

    fn runtime_control_capability() -> Capability {
        core::Capability::new(
            core::ServiceId::new(PROJECT_HOST_NAMESPACE, PROJECT_HOST_SERVICE_NAME),
            core::ServiceRole::ProjectHost,
        )
        .with_session(test_session_id())
        .with_audience(RUNTIME_HOST_AUDIENCE)
        .with_permissions([RUNTIME_CONTROL_PERMISSION])
    }

    fn staging_side_channel(name: &str) -> SideChannelHandle {
        let path = std::env::temp_dir().join(name);
        SideChannelHandle::staging_file(
            path.to_string_lossy(),
            128,
            vec![0x11; az_proto_core::SIDE_CHANNEL_BLAKE3_HASH_BYTES],
            std::env::consts::OS,
        )
    }

    fn test_node_type_catalog(catalog_version: u32) -> NodeTypeCatalog {
        let mut descriptor = NodeTypeDescriptor::new("azoth.test.print", 1, "Print String")
            .with_category_path(["Test".to_string(), "Debug".to_string()])
            .with_port(
                NodePortDescriptor::new(
                    NodePortId::new(1),
                    "value",
                    NodePortDirection::Input,
                    NodePortValue::Data {
                        schema_type: "core.string".to_string(),
                    },
                )
                .with_default_value(vnext::ReflectedValueEnvelope::typed_ron(
                    "core.string",
                    r#""hello""#,
                ))
                .with_layout(NodePortLayout::input().with_fixed_fraction(250)),
            )
            .with_port(
                NodePortDescriptor::new(
                    NodePortId::new(2),
                    "then",
                    NodePortDirection::Output,
                    NodePortValue::Execution,
                )
                .with_capacity(NodePortCapacity::Multiple),
            );
        descriptor.description = Some("Writes a string to the debug stream.".to_string());
        descriptor.capabilities.push(NodeCapability {
            id: "azoth.node.call".to_string(),
            markers: vec!["debug".to_string()],
        });
        descriptor.runtime_binding = Some(NodeRuntimeBinding::RustSymbol {
            package: "az-test-nodes".to_string(),
            symbol: "az_test_nodes::debug::print_string".to_string(),
            call_abi: RustNodeCallAbi::ContextSchedule,
        });
        descriptor.source_links.push(NodeSourceLink {
            package: Some("az-test-nodes".to_string()),
            module_path: Some("az_test_nodes::debug".to_string()),
            symbol_path: Some("print_string".to_string()),
            file: Some("src/debug.rs".to_string()),
            line: Some(12),
            column: Some(5),
            docs_url: None,
        });
        descriptor.tags.push("debug".to_string());

        NodeTypeCatalog::new(catalog_version, 1234, vec![descriptor])
    }

    fn test_graph_type_catalog(catalog_version: u32) -> GraphTypeCatalog {
        let packed_descriptor = GraphTypeDescriptor::runtime_compiled(
            "azoth.test.logic-graph",
            1,
            "Logic Graph",
            GraphSourceWorkflow::file("azoth.test.logic-graph.source", "azgraph.ron")
                .with_default_path_prefix("graphs"),
            GraphCompilerBackendDescriptor::packed_ir(
                "azoth.test.logic-graph.compiler",
                "azoth.graph.logic-ir/v1",
            )
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.packed-ir",
                "azoth.graph.logic-ir",
                RuntimeGraphExecutionStrategy::PackedIr,
            ),
        )
        .with_category_path(["Test".to_string(), "Logic".to_string()])
        .with_node_catalog(
            GraphNodeCatalogRequirement::new("azoth.test.nodes")
                .with_minimum_version(1)
                .with_required_hash(vec![7; blake3::OUT_LEN]),
        )
        .with_palette_policy(
            GraphPalettePolicy::default()
                .with_root_category("Logic")
                .with_required_node_capability("azoth.node.call"),
        )
        .with_tag("test");

        let generated_descriptor = GraphTypeDescriptor::runtime_compiled(
            "azoth.test.generated-graph",
            1,
            "Generated Graph",
            GraphSourceWorkflow::file("azoth.test.generated-graph.source", "azgraph.ron")
                .with_default_path_prefix("graphs/generated"),
            GraphCompilerBackendDescriptor::generated_rust_context_schedule(
                "azoth.test.generated-graph.compiler",
                "az-test-generated-graph",
                "az_test_generated_graph::compile",
            )
            .with_capability_marker("generated-rust"),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.generated-rust",
                "azoth.graph.generated-rust",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "az-test-generated-graph",
                    "az_test_generated_graph::execute",
                    "az_test_generated_graph::RuntimeContext",
                ),
            ),
        )
        .with_category_path(["Test".to_string(), "Generated".to_string()])
        .with_node_catalog(GraphNodeCatalogRequirement::new(
            "azoth.test.generated-nodes",
        ))
        .with_tag("generated-test");

        GraphTypeCatalog::new(
            catalog_version,
            1234,
            vec![packed_descriptor, generated_descriptor],
        )
    }

    fn test_graph_command_batch_snapshot() -> GraphCommandBatchSnapshot {
        let source = GraphNode::new(graph_node_id(1), "azoth.test.float", 1);
        let mut target = GraphNode::new(graph_node_id(2), "azoth.test.float", 1);
        target.layout = GraphNodeLayout { x: 10.0, y: 20.0 };
        target.input_values.insert(
            NodePortId::new(1),
            vnext::ReflectedValueEnvelope::typed_ron("f32", "1.5"),
        );
        let connection = GraphConnection::new(
            graph_connection_id(1),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        )
        .with_route(GraphConnectionRoute::orthogonal().with_anchor(
            GraphRouteAnchor::user_waypoint(graph_route_anchor_id(1), GraphPoint::new(44.0, 55.0)),
        ));
        let replacement_route = GraphConnectionRoute::orthogonal().with_anchor(
            GraphRouteAnchor::user_waypoint(graph_route_anchor_id(2), GraphPoint::new(60.0, 70.0))
                .with_outgoing_segment(GraphRouteSegmentConstraint::Fixed),
        );

        GraphCommandBatchSnapshot {
            document_id: DocumentId::new("graphs/test.visual.ron"),
            expected_revision: Some(DocumentRevision::new(7)),
            client_batch_id: "graph-batch-1".to_string(),
            commands: vec![
                GraphCommand::AddNode { node: source },
                GraphCommand::AddNode {
                    node: target.clone(),
                },
                GraphCommand::SetInputValue {
                    node_id: target.id,
                    port_id: NodePortId::new(1),
                    value: Some(vnext::ReflectedValueEnvelope::typed_ron("f32", "2.5")),
                },
                GraphCommand::MoveNode {
                    node_id: target.id,
                    layout: GraphNodeLayout { x: 30.0, y: 40.0 },
                },
                GraphCommand::Connect {
                    connection: connection.clone(),
                },
                GraphCommand::SetConnectionRoute {
                    connection_id: connection.id,
                    route: replacement_route,
                },
                GraphCommand::Disconnect {
                    connection_id: connection.id,
                },
                GraphCommand::UpsertComment {
                    comment: GraphComment {
                        id: graph_comment_id(1),
                        text: "logic note".to_string(),
                        bounds: GraphCommentBounds {
                            x: 1.0,
                            y: 2.0,
                            width: 300.0,
                            height: 120.0,
                        },
                    },
                },
                GraphCommand::RemoveComment {
                    comment_id: graph_comment_id(1),
                },
            ],
        }
    }

    fn test_graph_document_snapshot() -> GraphDocumentSnapshot {
        let source = GraphNode::new(graph_node_id(10), "azoth.test.float", 1);
        let mut target = GraphNode::new(graph_node_id(11), "azoth.test.float", 1);
        target.layout = GraphNodeLayout { x: 360.0, y: 80.0 };
        let connection = GraphConnection::new(
            graph_connection_id(10),
            GraphPortRef::new(source.id, NodePortId::new(2)),
            GraphPortRef::new(target.id, NodePortId::new(1)),
        )
        .with_route(
            GraphConnectionRoute::orthogonal().with_anchor(
                GraphRouteAnchor::user_waypoint(
                    graph_route_anchor_id(10),
                    GraphPoint::new(180.0, 96.0),
                )
                .with_outgoing_segment(GraphRouteSegmentConstraint::Fixed),
            ),
        );
        let mut document = VisualGraphDocument::new("azoth.graph.test");
        document.required_catalog_hash =
            Some(vec![0x42; az_proto_core::SIDE_CHANNEL_BLAKE3_HASH_BYTES]);
        document.nodes = vec![source, target];
        document.connections = vec![connection];
        document.comments = vec![GraphComment {
            id: graph_comment_id(10),
            text: "route waypoint note".to_string(),
            bounds: GraphCommentBounds {
                x: 40.0,
                y: 120.0,
                width: 320.0,
                height: 96.0,
            },
        }];

        GraphDocumentSnapshot {
            document_id: DocumentId::new("graphs/test.visual.ron"),
            revision: DocumentRevision::new(8),
            document,
        }
    }

    fn graph_node_id(value: u128) -> GraphNodeId {
        GraphNodeId::new(Uuid::from_u128(value))
    }

    fn graph_connection_id(value: u128) -> GraphConnectionId {
        GraphConnectionId::new(Uuid::from_u128(value))
    }

    fn graph_route_anchor_id(value: u128) -> GraphRouteAnchorId {
        GraphRouteAnchorId::new(Uuid::from_u128(value))
    }

    fn graph_comment_id(value: u128) -> GraphCommentId {
        GraphCommentId::new(Uuid::from_u128(value))
    }

    #[test]
    fn node_type_catalog_result_binds_request_capability_to_side_channel() {
        let capability = project_request_capability(PROJECT_NODE_CATALOG_PERMISSION);
        let snapshot = staging_side_channel("node-type-catalog.capnp.packed");

        let mut message = message::Builder::new_default();
        write_node_type_catalog_result(
            &snapshot,
            &capability,
            message
                .init_root::<project_capnp::project_host::node_type_catalog_results::Builder<'_>>(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<project_capnp::project_host::node_type_catalog_results::Reader<'_>>()
            .unwrap();
        let result = read_node_type_catalog_result_for_capability(reader, &capability).unwrap();

        assert_eq!(result.snapshot.capability.as_ref(), Some(&capability));
    }

    #[test]
    fn node_type_catalog_request_rejects_schema_only_capability() {
        let capability = project_request_capability(PROJECT_SCHEMA_PERMISSION);
        let request = ProjectHostCapabilityRequest { capability };
        let mut message = message::Builder::new_default();

        let error = write_node_type_catalog_request(
            &request,
            message
                .init_root::<project_capnp::project_host::node_type_catalog_params::Builder<'_>>(),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("capability missing `project.node.catalog`"),
            "{error}"
        );
    }

    #[test]
    fn graph_type_catalog_result_binds_request_capability_to_side_channel() {
        let capability = project_request_capability(PROJECT_GRAPH_CATALOG_PERMISSION);
        let snapshot = staging_side_channel("graph-type-catalog.capnp.packed");

        let mut message = message::Builder::new_default();
        write_graph_type_catalog_result(
            &snapshot,
            &capability,
            message
                .init_root::<project_capnp::project_host::graph_type_catalog_results::Builder<'_>>(
                ),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<project_capnp::project_host::graph_type_catalog_results::Reader<'_>>()
            .unwrap();
        let result = read_graph_type_catalog_result_for_capability(reader, &capability).unwrap();

        assert_eq!(result.snapshot.capability.as_ref(), Some(&capability));
    }

    #[test]
    fn graph_type_catalog_request_rejects_schema_only_capability() {
        let capability = project_request_capability(PROJECT_SCHEMA_PERMISSION);
        let request = ProjectHostCapabilityRequest { capability };
        let mut message = message::Builder::new_default();

        let error = write_graph_type_catalog_request(
            &request,
            message
                .init_root::<project_capnp::project_host::graph_type_catalog_params::Builder<'_>>(),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("capability missing `project.graph.catalog`"),
            "{error}"
        );
    }

    #[test]
    fn node_source_link_request_binds_to_source_navigation_capability() {
        let capability = project_request_capability(PROJECT_SOURCE_NAVIGATION_PERMISSION);
        let request = NodeSourceLinkRequest {
            capability,
            source_link: NodeSourceLink::rust_symbol(
                "az-proto-project",
                "az_proto_project::tests",
                "az_proto_project::tests::node",
                "src/lib.rs",
                12,
                5,
            ),
        };
        let mut message = message::Builder::new_default();

        write_node_source_link_request(
            &request,
            message
                .init_root::<
                    project_capnp::project_host::resolve_node_source_link_params::Builder<'_>,
                >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::resolve_node_source_link_params::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_node_source_link_request(reader).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn node_source_link_request_rejects_document_read_capability() {
        let request = NodeSourceLinkRequest {
            capability: project_request_capability(PROJECT_DOCUMENT_READ_PERMISSION),
            source_link: NodeSourceLink::rust_symbol(
                "az-proto-project",
                "az_proto_project::tests",
                "az_proto_project::tests::node",
                "src/lib.rs",
                12,
                5,
            ),
        };
        let mut message = message::Builder::new_default();

        let error = write_node_source_link_request(
            &request,
            message
                .init_root::<
                    project_capnp::project_host::resolve_node_source_link_params::Builder<'_>,
                >(),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains(PROJECT_SOURCE_NAVIGATION_PERMISSION),
            "{error}"
        );
    }

    #[test]
    fn node_source_link_target_round_trips_package_resolution() {
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "az-proto-project",
                "az_proto_project::tests",
                "az_proto_project::tests::node",
                "src/lib.rs",
                12,
                5,
            ),
            resolved_path: Some("workspace/crates/az/proto/project/src/lib.rs".to_string()),
            package_id: Some("azoth.proto-project".to_string()),
            package_root: Some("crates/az/proto/project".to_string()),
            path_kind: NodeSourceLinkPathKind::PackageRelative,
            exists: true,
        };
        let mut message = message::Builder::new_default();

        write_node_source_link_target(
            &target,
            message
                .init_root::<
                    project_capnp::project_host::resolve_node_source_link_results::Builder<'_>,
                >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::resolve_node_source_link_results::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_node_source_link_target(reader).unwrap();
        assert_eq!(decoded, target);
    }

    #[test]
    fn node_source_link_target_rejects_missing_resolved_path() {
        let target = NodeSourceLinkTarget {
            source_link: NodeSourceLink::rust_symbol(
                "az-proto-project",
                "az_proto_project::tests",
                "az_proto_project::tests::node",
                "src/lib.rs",
                12,
                5,
            ),
            resolved_path: None,
            package_id: None,
            package_root: None,
            path_kind: NodeSourceLinkPathKind::WorkspaceRelative,
            exists: false,
        };
        let mut message = message::Builder::new_default();

        let error = write_node_source_link_target(
            &target,
            message
                .init_root::<
                    project_capnp::project_host::resolve_node_source_link_results::Builder<'_>,
                >(),
        )
        .unwrap_err();

        assert!(error.to_string().contains("resolved path"), "{error}");
    }

    #[test]
    fn node_type_catalog_snapshot_round_trips_descriptors() {
        let catalog = test_node_type_catalog(NODE_TYPE_CATALOG_SNAPSHOT_VERSION);

        let bytes = encode_node_type_catalog_snapshot(&catalog).unwrap();
        let decoded = decode_node_type_catalog_snapshot_packed(&bytes).unwrap();

        assert_eq!(decoded, catalog);
    }

    #[test]
    fn graph_type_catalog_snapshot_round_trips_descriptors() {
        let catalog = test_graph_type_catalog(GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION);

        let bytes = encode_graph_type_catalog_snapshot(&catalog).unwrap();
        let decoded = decode_graph_type_catalog_snapshot_packed(&bytes).unwrap();

        assert_eq!(decoded, catalog);
        let generated = decoded
            .graph_types
            .iter()
            .find(|graph_type| graph_type.id.as_str() == "azoth.test.generated-graph")
            .expect("generated graph fixture should round-trip");
        let backend = generated.compiler_backend.as_ref().unwrap();
        assert!(matches!(
            backend.kind,
            GraphCompilerBackendKind::GeneratedRust {
                abi: GeneratedRustGraphAbi::ContextSchedule,
                ..
            }
        ));
    }

    #[test]
    fn graph_type_catalog_snapshot_rejects_wrong_snapshot_version() {
        let catalog = test_graph_type_catalog(GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION + 1);

        let bytes = encode_graph_type_catalog_snapshot(&catalog).unwrap();
        let error = decode_graph_type_catalog_snapshot_packed(&bytes).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported graph type catalog snapshot version"),
            "{error}"
        );
    }

    #[test]
    fn graph_type_catalog_side_channel_loads_verified_snapshot() {
        let catalog = test_graph_type_catalog(GRAPH_TYPE_CATALOG_SNAPSHOT_VERSION);
        let bytes = encode_graph_type_catalog_snapshot(&catalog).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("graph-type-catalog.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            blake3::hash(&bytes).as_bytes().to_vec(),
            std::env::consts::OS,
        );

        let decoded = load_graph_type_catalog_side_channel(&handle).unwrap();

        assert_eq!(decoded, catalog);
    }

    #[test]
    fn node_type_catalog_snapshot_rejects_wrong_snapshot_version() {
        let catalog = test_node_type_catalog(NODE_TYPE_CATALOG_SNAPSHOT_VERSION + 1);

        let bytes = encode_node_type_catalog_snapshot(&catalog).unwrap();
        let error = decode_node_type_catalog_snapshot_packed(&bytes).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported node type catalog snapshot version"),
            "{error}"
        );
    }

    #[test]
    fn node_type_catalog_side_channel_loads_verified_snapshot() {
        let catalog = test_node_type_catalog(NODE_TYPE_CATALOG_SNAPSHOT_VERSION);
        let bytes = encode_node_type_catalog_snapshot(&catalog).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("node-type-catalog.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            blake3::hash(&bytes).as_bytes().to_vec(),
            std::env::consts::OS,
        );

        let decoded = load_node_type_catalog_side_channel(&handle).unwrap();

        assert_eq!(decoded, catalog);
    }

    #[test]
    fn graph_command_batch_snapshot_round_trips_commands() {
        let batch = test_graph_command_batch_snapshot();

        let bytes = encode_graph_command_batch_snapshot(&batch).unwrap();
        let decoded = decode_graph_command_batch_snapshot_packed(&bytes).unwrap();

        assert_eq!(decoded, batch);
    }

    #[test]
    fn graph_command_batch_snapshot_rejects_wrong_snapshot_version() {
        let mut message = message::Builder::new_default();
        let mut root =
            message.init_root::<project_capnp::graph_command_batch_snapshot::Builder<'_>>();
        root.set_snapshot_version(GRAPH_COMMAND_BATCH_SNAPSHOT_VERSION + 1);
        write_document_id("graphs/test.visual.ron", root.reborrow().init_document());
        root.set_client_batch_id("graph-batch-1");
        root.set_no_expected_revision(());
        root.reborrow().init_commands(0);
        let mut bytes = Vec::new();
        serialize_packed::write_message(&mut bytes, &message).unwrap();

        let error = decode_graph_command_batch_snapshot_packed(&bytes).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("unsupported graph command batch snapshot version"),
            "{error}"
        );
    }

    #[test]
    fn graph_command_batch_side_channel_loads_verified_snapshot() {
        let batch = test_graph_command_batch_snapshot();
        let bytes = encode_graph_command_batch_snapshot(&batch).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("graph-command-batch.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            blake3::hash(&bytes).as_bytes().to_vec(),
            std::env::consts::OS,
        );

        let decoded = load_graph_command_batch_side_channel(&handle).unwrap();

        assert_eq!(decoded, batch);
    }

    #[test]
    fn graph_command_batch_rejects_empty_command_list() {
        let batch = GraphCommandBatchSnapshot {
            document_id: DocumentId::new("graphs/test.visual.ron"),
            expected_revision: None,
            client_batch_id: "graph-batch-1".to_string(),
            commands: Vec::new(),
        };

        let error = encode_graph_command_batch_snapshot(&batch).unwrap_err();

        assert!(error.to_string().contains("command count"), "{error}");
    }

    #[test]
    fn graph_command_status_snapshot_round_trips_rejection() {
        let status = GraphCommandStatusSnapshot {
            document_id: DocumentId::new("graphs/test.visual.ron"),
            client_batch_id: "graph-batch-1".to_string(),
            applied_command_count: 2,
            outcome: GraphCommandStatusOutcome::Rejected {
                command_index: Some(2),
                reason: "port schema mismatch".to_string(),
            },
            diagnostics: vec![GraphCommandDiagnostic {
                command_index: Some(2),
                severity: GraphDiagnosticSeverity::Error,
                message: "output `core.f32` cannot connect to input `core.string`".to_string(),
            }],
        };

        let bytes = encode_graph_command_status_snapshot(&status).unwrap();
        let decoded = decode_graph_command_status_snapshot_packed(&bytes).unwrap();

        assert_eq!(decoded, status);
    }

    #[test]
    fn graph_document_snapshot_round_trips_document_state() {
        let snapshot = test_graph_document_snapshot();

        let bytes = encode_graph_document_snapshot(&snapshot).unwrap();
        let decoded = decode_graph_document_snapshot_packed(&bytes).unwrap();

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn graph_document_snapshot_side_channel_loads_verified_snapshot() {
        let snapshot = test_graph_document_snapshot();
        let bytes = encode_graph_document_snapshot(&snapshot).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("graph-document-snapshot.capnp.packed");
        std::fs::write(&path, &bytes).unwrap();
        let handle = SideChannelHandle::staging_file(
            path.to_string_lossy(),
            bytes.len() as u64,
            blake3::hash(&bytes).as_bytes().to_vec(),
            std::env::consts::OS,
        );

        let decoded = load_graph_document_side_channel(&handle).unwrap();

        assert_eq!(decoded, snapshot);
    }

    #[test]
    fn save_graph_document_request_binds_to_document_write_capability() {
        let capability = project_request_capability(PROJECT_DOCUMENT_WRITE_PERMISSION);
        let request = ProjectDocumentRequest {
            capability,
            document_id: DocumentId::new("graphs/save.visual.ron"),
        };
        let mut message = message::Builder::new_default();
        write_save_graph_document_request(
            &request,
            message
                .init_root::<project_capnp::project_host::save_graph_document_params::Builder<'_>>(
                ),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::save_graph_document_params::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_save_graph_document_request(reader).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn save_graph_document_result_round_trips_saved_source_payload_record() {
        let result = SaveDocumentResult {
            revision: DocumentRevision::new(3),
            saved: SavedDocument {
                document_id: DocumentId::new("graphs/save.visual.ron"),
                revision: DocumentRevision::new(3),
                source_path: "graphs/save.visual.ron".to_string(),
                schema_type: "azoth.graph.test".to_string(),
                content_hash: vec![0x7a; 32],
                byte_length: 2048,
            },
        };
        let mut message = message::Builder::new_default();
        write_save_graph_document_result(
            &result,
            message
                .init_root::<project_capnp::project_host::save_graph_document_results::Builder<'_>>(
                ),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::save_graph_document_results::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_save_graph_document_result(reader).unwrap();

        assert_eq!(decoded, result);
    }

    #[test]
    fn create_graph_document_request_binds_to_document_write_capability() {
        let capability = project_request_capability(PROJECT_DOCUMENT_WRITE_PERMISSION);
        let request = CreateGraphDocumentRequest {
            capability,
            document_id: DocumentId::new("graphs/new.visual.ron"),
            graph_type: "azoth.graph.test".to_string(),
        };
        let mut message = message::Builder::new_default();
        write_create_graph_document_request(
            &request,
            message.init_root::<
                project_capnp::project_host::create_graph_document_params::Builder<'_>,
            >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::create_graph_document_params::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_create_graph_document_request(reader).unwrap();

        assert_eq!(decoded, request);
    }

    #[test]
    fn graph_document_snapshot_result_binds_snapshot_handle_to_read_capability() {
        let capability = project_request_capability(PROJECT_DOCUMENT_READ_PERMISSION);
        let snapshot = staging_side_channel("graph-document-snapshot.capnp.packed");
        let mut message = message::Builder::new_default();
        write_graph_document_snapshot_result(
            &snapshot,
            &capability,
            message.init_root::<
                project_capnp::project_host::graph_document_snapshot_results::Builder<'_>,
            >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::graph_document_snapshot_results::Reader<'_>,
            >()
            .unwrap();
        let decoded =
            read_graph_document_snapshot_result_for_capability(reader, &capability).unwrap();

        assert_eq!(decoded.snapshot.capability, Some(capability));
    }

    #[test]
    fn create_graph_document_result_binds_snapshot_handle_to_write_capability() {
        let capability = project_request_capability(PROJECT_DOCUMENT_WRITE_PERMISSION);
        let snapshot = staging_side_channel("graph-document-created.capnp.packed");
        let mut message = message::Builder::new_default();
        write_create_graph_document_result(
            &snapshot,
            &capability,
            message.init_root::<
                project_capnp::project_host::create_graph_document_results::Builder<'_>,
            >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::create_graph_document_results::Reader<'_>,
            >()
            .unwrap();
        let decoded =
            read_create_graph_document_result_for_capability(reader, &capability).unwrap();

        assert_eq!(decoded.snapshot.capability, Some(capability));
    }

    #[test]
    fn apply_graph_commands_request_binds_batch_handle_to_edit_capability() {
        let capability = project_request_capability(PROJECT_EDIT_PERMISSION);
        let request = GraphCommandBatchRequest {
            capability: capability.clone(),
            batch: staging_side_channel("graph-command-batch.capnp.packed"),
        };
        let mut message = message::Builder::new_default();
        write_apply_graph_commands_request(
            &request,
            message
                .init_root::<project_capnp::project_host::apply_graph_commands_params::Builder<'_>>(
                ),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::apply_graph_commands_params::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_apply_graph_commands_request(reader).unwrap();

        assert_eq!(decoded.capability, capability);
        assert_eq!(decoded.batch.capability, Some(capability));
    }

    #[test]
    fn apply_graph_commands_request_decode_rejects_batch_capability_mismatch() {
        let capability = project_request_capability(PROJECT_EDIT_PERMISSION);
        let wrong = project_request_capability(PROJECT_EDIT_PERMISSION).with_token_hash([0x99]);
        let batch = staging_side_channel("graph-command-batch-mismatch.capnp.packed")
            .with_capability(wrong);
        let mut message = message::Builder::new_default();
        let mut root = message
            .init_root::<project_capnp::project_host::apply_graph_commands_params::Builder<'_>>();
        core::Capability::to_capnp(&capability, root.reborrow().init_capability()).unwrap();
        core::SideChannelHandle::to_capnp(&batch, root.reborrow().init_batch()).unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::apply_graph_commands_params::Reader<'_>,
            >()
            .unwrap();
        let error = read_apply_graph_commands_request(reader).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("not bound to the expected command capability")
        );
    }

    #[test]
    fn apply_graph_commands_result_binds_status_handle_to_edit_capability() {
        let capability = project_request_capability(PROJECT_EDIT_PERMISSION);
        let status = staging_side_channel("graph-command-status.capnp.packed");
        let mut message = message::Builder::new_default();
        write_apply_graph_commands_result(
            &status,
            &capability,
            message.init_root::<
                project_capnp::project_host::apply_graph_commands_results::Builder<'_>,
            >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::apply_graph_commands_results::Reader<'_>,
            >()
            .unwrap();
        let decoded = read_apply_graph_commands_result_for_capability(reader, &capability).unwrap();

        assert_eq!(decoded.snapshot.capability, Some(capability));
    }

    #[test]
    fn runtime_launch_snapshot_result_encode_rejects_project_capability() {
        let snapshot = staging_side_channel("runtime-launch-project-capability.capnp.packed");
        let mut message = message::Builder::new_default();
        let error = write_runtime_launch_snapshot_result(
            &snapshot,
            &project_request_capability(PROJECT_SCHEMA_PERMISSION),
            message.init_root::<
                project_capnp::project_host::runtime_launch_snapshot_results::Builder<'_>,
            >(),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("runtime launch capability must be issued"),
            "{error}"
        );
    }

    #[test]
    fn runtime_launch_snapshot_result_decode_rejects_capability_mismatch() {
        let expected = runtime_control_capability();
        let wrong = expected.clone().with_token_hash([0x99]);
        let snapshot = staging_side_channel("runtime-launch.capnp.packed");
        let mut message = message::Builder::new_default();
        write_runtime_launch_snapshot_result(
            &snapshot,
            &wrong,
            message.init_root::<
                project_capnp::project_host::runtime_launch_snapshot_results::Builder<'_>,
            >(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<
                project_capnp::project_host::runtime_launch_snapshot_results::Reader<'_>,
            >()
            .unwrap();
        let error =
            read_runtime_launch_snapshot_result_for_capability(reader, &expected).unwrap_err();

        assert!(error.to_string().contains("side-channel handle"));
    }

    #[test]
    fn saved_document_round_trips_source_mutation_metadata() {
        let expected = SavedDocument {
            document_id: DocumentId::new("prefabs/door.prefab.ron"),
            revision: DocumentRevision::new(4),
            source_path: "prefabs/door.prefab.ron".to_string(),
            schema_type: "az.test.Prefab".to_string(),
            content_hash: vec![0xab; 32],
            byte_length: 4096,
        };

        let mut message = message::Builder::new_default();
        write_saved_document(
            &expected,
            message.init_root::<project_capnp::saved_document::Builder<'_>>(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<project_capnp::saved_document::Reader<'_>>()
            .unwrap();
        let decoded = read_saved_document(reader).unwrap();

        assert_eq!(decoded, expected);
    }

    #[test]
    fn runtime_launch_snapshot_request_round_trips_runtime_context() {
        let expected = RuntimeLaunchSnapshotRequest {
            capability: project_request_capability(PROJECT_RUNTIME_LAUNCH_PERMISSION),
            runtime_launch_capability: runtime_control_capability(),
            role: RuntimeRole::EditorWorld,
            project_id: "local.runtime".to_string(),
            session_id: test_session_id(),
            session_slug: "lighting".to_string(),
            project_root: "projects/runtime".to_string(),
            workspace_path: "projects/runtime/.azoth/workspaces/lighting".to_string(),
            workspace_id: 12,
            include_unsaved_journal: true,
            launch_profile: "editor".to_string(),
            asset_source_roots: vec![RuntimeAssetSourceRoot {
                workspace_root_id: 22,
                workspace_id: 12,
                root_id: 34,
                owner_id: "local.runtime".to_string(),
                source_root: "projects/runtime/.azoth/workspaces/lighting/assets".to_string(),
                display_name: "Project Assets".to_string(),
                portable_key: "project:local.runtime:assets".to_string(),
                output_prefix: String::new(),
                is_root: true,
            }],
            asset_package_roots: vec![RuntimeAssetPackageRoot {
                profile: "pc-dev".to_string(),
                asset_platform: "pc".to_string(),
                container: az_proto_runtime::RuntimeAssetPackageContainer::AzPack,
                mount_root: "projects/runtime/target/azoth/packages/pc-dev/lighting/azpack"
                    .to_string(),
                payload_path: "projects/runtime/target/azoth/packages/pc-dev/lighting/azpack"
                    .to_string(),
                catalog_path:
                    "projects/runtime/target/azoth/packages/pc-dev/lighting/azpack/assetcatalog.bin"
                        .to_string(),
                release_id: "0".repeat(az_proto_runtime::RUNTIME_PACKAGE_RELEASE_ID_HEX_LEN),
            }],
        };

        let mut message = message::Builder::new_default();
        write_runtime_launch_snapshot_request(
            &expected,
            message.init_root::<project_capnp::runtime_launch_snapshot_request::Builder<'_>>(),
        )
        .unwrap();

        let reader = message
            .get_root_as_reader::<project_capnp::runtime_launch_snapshot_request::Reader<'_>>()
            .unwrap();

        assert_eq!(
            read_runtime_launch_snapshot_request(reader).unwrap(),
            expected
        );
    }

    #[test]
    fn saved_document_encode_rejects_bad_hash_length() {
        let saved = SavedDocument {
            document_id: DocumentId::new("prefabs/door.prefab.ron"),
            revision: DocumentRevision::new(4),
            source_path: "prefabs/door.prefab.ron".to_string(),
            schema_type: "az.test.Prefab".to_string(),
            content_hash: vec![0xab; 31],
            byte_length: 4096,
        };

        let mut message = message::Builder::new_default();
        let error = write_saved_document(
            &saved,
            message.init_root::<project_capnp::saved_document::Builder<'_>>(),
        )
        .unwrap_err();

        assert!(
            error.to_string().contains("content hash must be 32 bytes"),
            "{error}"
        );
    }

    #[test]
    fn runtime_launch_snapshot_request_encode_rejects_missing_project_assets_root() {
        let request = RuntimeLaunchSnapshotRequest {
            capability: project_request_capability(PROJECT_RUNTIME_LAUNCH_PERMISSION),
            runtime_launch_capability: runtime_control_capability(),
            role: RuntimeRole::EditorWorld,
            project_id: "local.runtime".to_string(),
            session_id: test_session_id(),
            session_slug: "lighting".to_string(),
            project_root: "projects/runtime".to_string(),
            workspace_path: "projects/runtime/.azoth/workspaces/lighting".to_string(),
            workspace_id: 12,
            include_unsaved_journal: true,
            launch_profile: "editor".to_string(),
            asset_source_roots: vec![RuntimeAssetSourceRoot {
                workspace_root_id: 22,
                workspace_id: 12,
                root_id: 34,
                owner_id: "azoth.physics".to_string(),
                source_root: "projects/runtime/.azoth/workspaces/lighting/gems/physics/assets"
                    .to_string(),
                display_name: "Physics Assets".to_string(),
                portable_key: "gem:azoth.physics:assets".to_string(),
                output_prefix: "gems/azoth.physics".to_string(),
                is_root: false,
            }],
            asset_package_roots: Vec::new(),
        };

        let mut message = message::Builder::new_default();
        let error = write_runtime_launch_snapshot_request(
            &request,
            message.init_root::<project_capnp::runtime_launch_snapshot_request::Builder<'_>>(),
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("missing DB-owned project assets root"),
            "{error}"
        );
    }
}
