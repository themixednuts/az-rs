//! Editor-side visual graph UI adapter.
//!
//! This module is the controller/projection boundary a GPUI graph panel should
//! render through. It does not own graph semantics or persistence: project-host
//! owns graph documents, `az-node-graph` owns validation, and
//! `az-graph-layout` owns geometry jobs. The adapter turns UI intents into
//! project-host graph command batches and refreshes its local projection only
//! after project-host accepts a batch.

use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

use az_core::reflect::ReflectedValueEnvelope;
use az_editor_ui::panels::{
    Console, EditorGraphDocumentProjection, GraphBuildJobProjectionData, GraphBuildJobStatusData,
    GraphBuildSourceStatusData, GraphBuildStatusProjectionData, GraphCommentProjectionData,
    GraphCompilerBackendKindProjectionData, GraphCompilerBackendProjectionData,
    GraphConnectionProjectionData, GraphCreationCatalogProjectionData,
    GraphDocumentListItemProjectionData, GraphDocumentListProjectionData,
    GraphDocumentProjectionData, GraphGeneratedRustAbiProjectionData,
    GraphInputValueProjectionData, GraphNodePaletteItemData, GraphNodePaletteProjectionData,
    GraphNodeProjectionData, GraphNodeRuntimeBindingProjectionData,
    GraphNodeSourceLinkProjectionData, GraphPointProjectionData, GraphPortDirectionData,
    GraphPortProjectionData, GraphPortSideData, GraphRouteAnchorKindData,
    GraphRouteAnchorProjectionData, GraphRuntimeExecutionStrategyProjectionData,
    GraphRustCallResultProjectionData, GraphRustNodeCallAbiProjectionData,
    GraphTypeCreationProjectionData, LogLevel,
};
use az_graph_layout::{
    DefaultGraphLayoutSolver, GraphAutoLayoutOptions, GraphGeometrySnapshot, GraphLayoutError,
    GraphLayoutOperation, GraphLayoutRequest, GraphLayoutSolver, GraphLayoutTuning, GraphRect,
    GraphRouteOptions, GraphSpatialEntry, GraphSpatialIndex, graph_connection_route_points,
    graph_node_bounds, graph_port_anchor,
};
use az_node_graph::{
    GeneratedRustGraphAbi, GraphCommand, GraphCommandApplyError, GraphComment, GraphCommentBounds,
    GraphCommentId, GraphCompilerBackendDescriptor, GraphCompilerBackendKind, GraphConnection,
    GraphConnectionId, GraphConnectionRoute, GraphExecutionMode, GraphNode, GraphNodeId,
    GraphNodeLayout, GraphPalettePolicy, GraphPoint, GraphPortRef, GraphRouteAnchorId,
    GraphRouteAnchorKind, GraphSourceWorkflowKind, GraphTypeCatalog, GraphTypeDescriptor,
    GraphTypeId, NodePortDescriptor, NodePortDirection, NodePortId, NodePortSide, NodePortValue,
    NodeRuntimeBinding, NodeSourceLink, NodeTypeCatalog, NodeTypeDescriptor, NodeTypeId,
    RuntimeGraphExecutionStrategy, RustCallResult, RustDataflowOutput, RustDataflowParameterSource,
    RustNodeCallAbi, RustValuePassing, VisualGraphValidationError,
};
use az_proto_asset::{AssetRootScope, WorkspaceEntry};
use az_proto_core::ServiceDescriptor;
use az_proto_project::{
    DocumentId, DocumentRevision, GraphCommandBatchSnapshot, GraphCommandStatusOutcome,
    GraphCommandStatusSnapshot, GraphDocumentSnapshot,
};
use az_proto_session::{
    SaveGraphDocumentResult, SessionWorkspaceStatus as ProtoSessionWorkspaceStatus,
};
use gpui::App;
use thiserror::Error;
use tracing::{error, info};
use uuid::Uuid;

use crate::asset_processor::{
    AssetProcessorClient, WORKSPACE_ENTRY_PAGE_SIZE, asset_processor_descriptor_from_status,
};
use crate::attach::EditorAttachSession;
use crate::error::{EditorError, EditorResult};
use crate::project_host::ProjectHostClient;
use crate::session_supervisor::SessionSupervisorClient;
use crate::settings::SettingsStore;
use crate::source_navigation::{default_source_navigation_settings, source_navigation_intent};

#[derive(Debug, Error)]
pub enum EditorGraphUiAdapterError {
    #[error("graph UI adapter received an invalid graph document: {0}")]
    InvalidDocument(#[from] VisualGraphValidationError),
    #[error("graph UI command batch failed local graph validation: {0}")]
    InvalidCommandBatch(#[from] GraphCommandApplyError),
    #[error("graph layout solver failed: {0}")]
    Layout(#[from] GraphLayoutError),
    #[error("graph UI batch id cannot be empty")]
    EmptyClientBatchId,
    #[error("graph UI command references unknown connection {connection_id}")]
    UnknownConnection { connection_id: GraphConnectionId },
    #[error(
        "graph UI command references unknown route anchor {anchor_id} on connection {connection_id}"
    )]
    UnknownRouteAnchor {
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
    },
    #[error("graph UI command references unknown comment {comment_id}")]
    UnknownComment { comment_id: GraphCommentId },
    #[error("graph UI status mismatch: {0}")]
    StatusMismatch(String),
}

pub type EditorGraphUiResult<T> = Result<T, EditorGraphUiAdapterError>;

#[derive(Debug, Error)]
pub enum EditorGraphCreationError {
    #[error("graph type `{graph_type}` does not declare a default source extension")]
    MissingDefaultExtension { graph_type: String },
    #[error("graph document name cannot be empty")]
    EmptyDocumentName,
    #[error("graph document name `{name}` must be a file stem, not a path")]
    InvalidDocumentName { name: String },
}

pub type EditorGraphCreationResult<T> = Result<T, EditorGraphCreationError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorGraphCreationCatalog {
    pub graph_types: Vec<EditorGraphTypeCreationData>,
}

impl EditorGraphCreationCatalog {
    #[must_use]
    pub const fn new(graph_types: Vec<EditorGraphTypeCreationData>) -> Self {
        Self { graph_types }
    }

    #[must_use]
    pub fn graph_type(&self, graph_type: &str) -> Option<&EditorGraphTypeCreationData> {
        self.graph_types
            .iter()
            .find(|candidate| candidate.graph_type == graph_type)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorGraphTypeCreationData {
    pub graph_type: String,
    pub version: u32,
    pub label: String,
    pub category: String,
    pub source_workflow_id: String,
    pub source_workflow_kind: EditorGraphSourceWorkflowKindData,
    pub default_path_prefix: String,
    pub default_extension: String,
    pub compiler_backend: Option<EditorGraphCompilerBackendData>,
    pub runtime_product_asset_type: Option<String>,
    pub runtime_product_kind: Option<String>,
    pub runtime_product_streamable: Option<bool>,
    pub runtime_product_diffable_chunks: Option<bool>,
    pub runtime_execution_strategy: Option<EditorGraphRuntimeExecutionStrategyData>,
    pub runtime_compiled: bool,
    pub editor_interpreted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorGraphSourceWorkflowKindData {
    ProjectDocument,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorGraphCompilerBackendData {
    pub id: String,
    pub kind: EditorGraphCompilerBackendKindData,
    pub capability_markers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorGraphCompilerBackendKindData {
    GeneratedRust {
        package: String,
        entry_symbol: String,
        abi: EditorGeneratedRustGraphAbiData,
    },
    PackedIr {
        ir_schema: String,
    },
    ShaderPipeline {
        pipeline_kind: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorGeneratedRustGraphAbiData {
    ContextSchedule,
    TypedDataflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorGraphRuntimeExecutionStrategyData {
    PackedIr,
    AotCompiledCode {
        language: String,
        package: String,
        entry_symbol: String,
        context_type: String,
    },
    HotReloadedCompiledModule {
        abi: String,
        entry_symbol: String,
    },
    ShaderPipeline {
        pipeline_kind: String,
    },
    External {
        kind: String,
        locator: String,
    },
}

#[must_use]
pub fn graph_creation_catalog_from_graph_type_catalog(
    catalog: &GraphTypeCatalog,
) -> EditorGraphCreationCatalog {
    let mut graph_types = catalog
        .graph_types
        .iter()
        .map(graph_type_creation_data)
        .collect::<Vec<_>>();
    graph_types.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.graph_type.cmp(&right.graph_type))
    });
    EditorGraphCreationCatalog::new(graph_types)
}

/// Builds the native document id a new graph document of `graph_type` is
/// created at, from the graph type's default path prefix and extension.
///
/// # Errors
///
/// Returns [`EditorGraphCreationError::EmptyDocumentName`] if `document_name`
/// trims to nothing, [`EditorGraphCreationError::InvalidDocumentName`] if it is
/// `.`, `..`, or carries a path separator or drive colon, and
/// [`EditorGraphCreationError::MissingDefaultExtension`] if `graph_type`
/// declares no non-empty default source extension.
pub fn graph_document_id_from_creation_data(
    graph_type: &EditorGraphTypeCreationData,
    document_name: &str,
) -> EditorGraphCreationResult<DocumentId> {
    let name = document_name.trim();
    if name.is_empty() {
        return Err(EditorGraphCreationError::EmptyDocumentName);
    }
    if name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains(':')
    {
        return Err(EditorGraphCreationError::InvalidDocumentName {
            name: name.to_string(),
        });
    }
    let extension = graph_type.default_extension.trim().trim_start_matches('.');
    if extension.is_empty() {
        return Err(EditorGraphCreationError::MissingDefaultExtension {
            graph_type: graph_type.graph_type.clone(),
        });
    }

    let prefix = graph_type
        .default_path_prefix
        .trim()
        .trim_matches('/')
        .trim_matches('\\');
    let document_id = if prefix.is_empty() {
        format!("{name}.{extension}")
    } else {
        format!("{prefix}/{name}.{extension}")
    };
    Ok(DocumentId::new(document_id))
}

fn graph_type_creation_data(graph_type: &GraphTypeDescriptor) -> EditorGraphTypeCreationData {
    let runtime_product = graph_type.runtime_product.as_ref();
    EditorGraphTypeCreationData {
        graph_type: graph_type.id.as_str().to_string(),
        version: graph_type.version,
        label: graph_type.display_name.clone(),
        category: graph_type.category_path.join("/"),
        source_workflow_id: graph_type.source_workflow.workflow_id.clone(),
        source_workflow_kind: match graph_type.source_workflow.kind {
            GraphSourceWorkflowKind::ProjectDocument => {
                EditorGraphSourceWorkflowKindData::ProjectDocument
            }
            GraphSourceWorkflowKind::File => EditorGraphSourceWorkflowKindData::File,
        },
        default_path_prefix: graph_type
            .source_workflow
            .default_path_prefix
            .clone()
            .unwrap_or_default(),
        default_extension: graph_type
            .source_workflow
            .default_extension
            .clone()
            .unwrap_or_default(),
        compiler_backend: graph_type
            .compiler_backend
            .as_ref()
            .map(graph_compiler_backend_data),
        runtime_product_asset_type: runtime_product.map(|product| product.asset_type.clone()),
        runtime_product_kind: runtime_product.map(|product| product.product_kind.clone()),
        runtime_product_streamable: runtime_product.map(|product| product.streamable),
        runtime_product_diffable_chunks: runtime_product.map(|product| product.diffable_chunks),
        runtime_execution_strategy: runtime_product
            .map(|product| graph_runtime_execution_strategy_data(&product.execution_strategy)),
        runtime_compiled: matches!(
            graph_type.execution_mode,
            GraphExecutionMode::RuntimeCompiled
                | GraphExecutionMode::RuntimeCompiledAndEditorInterpreted
        ),
        editor_interpreted: matches!(
            graph_type.execution_mode,
            GraphExecutionMode::EditorInterpreted
                | GraphExecutionMode::RuntimeCompiledAndEditorInterpreted
        ),
    }
}

fn graph_compiler_backend_data(
    backend: &GraphCompilerBackendDescriptor,
) -> EditorGraphCompilerBackendData {
    EditorGraphCompilerBackendData {
        id: backend.id.clone(),
        kind: graph_compiler_backend_kind_data(&backend.kind),
        capability_markers: backend.capability_markers.clone(),
    }
}

fn graph_compiler_backend_kind_data(
    kind: &GraphCompilerBackendKind,
) -> EditorGraphCompilerBackendKindData {
    match kind {
        GraphCompilerBackendKind::GeneratedRust {
            package,
            entry_symbol,
            abi,
        } => EditorGraphCompilerBackendKindData::GeneratedRust {
            package: package.clone(),
            entry_symbol: entry_symbol.clone(),
            abi: generated_rust_graph_abi_data(*abi),
        },
        GraphCompilerBackendKind::PackedIr { ir_schema } => {
            EditorGraphCompilerBackendKindData::PackedIr {
                ir_schema: ir_schema.clone(),
            }
        }
        GraphCompilerBackendKind::ShaderPipeline { pipeline_kind } => {
            EditorGraphCompilerBackendKindData::ShaderPipeline {
                pipeline_kind: pipeline_kind.clone(),
            }
        }
        GraphCompilerBackendKind::External { kind, locator } => {
            EditorGraphCompilerBackendKindData::External {
                kind: kind.clone(),
                locator: locator.clone(),
            }
        }
    }
}

const fn generated_rust_graph_abi_data(
    abi: GeneratedRustGraphAbi,
) -> EditorGeneratedRustGraphAbiData {
    match abi {
        GeneratedRustGraphAbi::ContextSchedule => EditorGeneratedRustGraphAbiData::ContextSchedule,
        GeneratedRustGraphAbi::TypedDataflow => EditorGeneratedRustGraphAbiData::TypedDataflow,
    }
}

fn graph_runtime_execution_strategy_data(
    strategy: &RuntimeGraphExecutionStrategy,
) -> EditorGraphRuntimeExecutionStrategyData {
    match strategy {
        RuntimeGraphExecutionStrategy::PackedIr => {
            EditorGraphRuntimeExecutionStrategyData::PackedIr
        }
        RuntimeGraphExecutionStrategy::AotCompiledCode {
            language,
            package,
            entry_symbol,
            context_type,
        } => EditorGraphRuntimeExecutionStrategyData::AotCompiledCode {
            language: language.clone(),
            package: package.clone(),
            entry_symbol: entry_symbol.clone(),
            context_type: context_type.clone(),
        },
        RuntimeGraphExecutionStrategy::HotReloadedCompiledModule { abi, entry_symbol } => {
            EditorGraphRuntimeExecutionStrategyData::HotReloadedCompiledModule {
                abi: abi.clone(),
                entry_symbol: entry_symbol.clone(),
            }
        }
        RuntimeGraphExecutionStrategy::ShaderPipeline { pipeline_kind } => {
            EditorGraphRuntimeExecutionStrategyData::ShaderPipeline {
                pipeline_kind: pipeline_kind.clone(),
            }
        }
        RuntimeGraphExecutionStrategy::External { kind, locator } => {
            EditorGraphRuntimeExecutionStrategyData::External {
                kind: kind.clone(),
                locator: locator.clone(),
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct EditorGraphUiAdapter {
    snapshot: GraphDocumentSnapshot,
    catalog: NodeTypeCatalog,
    geometry: GraphGeometrySnapshot,
    spatial_index: GraphSpatialIndex,
}

impl EditorGraphUiAdapter {
    /// Validates `snapshot` against `catalog` and builds the spatial index the
    /// panel queries.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::InvalidDocument`] if the snapshot's
    /// document does not validate against `catalog` — an unknown node type or
    /// version, or a connection or port the catalog does not describe.
    pub fn new(
        snapshot: GraphDocumentSnapshot,
        catalog: NodeTypeCatalog,
        geometry: GraphGeometrySnapshot,
    ) -> EditorGraphUiResult<Self> {
        snapshot.document.validate_against(&catalog)?;
        let spatial_index = GraphSpatialIndex::build(
            &snapshot.document,
            &catalog,
            &geometry,
            GraphLayoutTuning::default(),
        );
        Ok(Self {
            snapshot,
            catalog,
            geometry,
            spatial_index,
        })
    }

    #[must_use]
    pub const fn document_id(&self) -> &DocumentId {
        &self.snapshot.document_id
    }

    #[must_use]
    pub const fn revision(&self) -> DocumentRevision {
        self.snapshot.revision
    }

    #[must_use]
    pub const fn snapshot(&self) -> &GraphDocumentSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn catalog(&self) -> &NodeTypeCatalog {
        &self.catalog
    }

    #[must_use]
    pub const fn geometry(&self) -> &GraphGeometrySnapshot {
        &self.geometry
    }

    #[must_use]
    pub const fn spatial_index(&self) -> &GraphSpatialIndex {
        &self.spatial_index
    }

    #[must_use]
    pub fn query_rect(&self, rect: GraphRect) -> Vec<&GraphSpatialEntry> {
        self.spatial_index.query_rect(rect)
    }

    /// Adopts a project-host snapshot of the document this adapter already
    /// projects, then rebuilds the spatial index.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::StatusMismatch`] if `snapshot` names
    /// a different document than the adapter holds, or
    /// [`EditorGraphUiAdapterError::InvalidDocument`] if the incoming document
    /// does not validate against the adapter's node catalog.
    pub fn replace_snapshot(&mut self, snapshot: GraphDocumentSnapshot) -> EditorGraphUiResult<()> {
        if snapshot.document_id != self.snapshot.document_id {
            return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                "snapshot document `{}` does not match adapter document `{}`",
                snapshot.document_id.as_str(),
                self.snapshot.document_id.as_str()
            )));
        }
        snapshot.document.validate_against(&self.catalog)?;
        self.snapshot = snapshot;
        self.rebuild_spatial_index();
        Ok(())
    }

    pub fn replace_geometry(&mut self, geometry: GraphGeometrySnapshot) {
        self.geometry = geometry;
        self.rebuild_spatial_index();
    }

    pub fn set_node_bounds(&mut self, node_id: GraphNodeId, bounds: GraphRect) {
        self.geometry.node_bounds.insert(node_id, bounds);
        self.rebuild_spatial_index();
    }

    /// Plans a single-command batch that moves `node_id` to `layout`.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns.
    pub fn plan_move_node(
        &self,
        client_batch_id: impl Into<String>,
        node_id: GraphNodeId,
        layout: GraphNodeLayout,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::MoveNode { node_id, layout }],
        )
    }

    /// Plans a single-command batch that removes `node_id` and its edges.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns.
    pub fn plan_remove_node(
        &self,
        client_batch_id: impl Into<String>,
        node_id: GraphNodeId,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(client_batch_id, vec![GraphCommand::RemoveNode { node_id }])
    }

    /// Plans a single-command batch that connects `from` to `to` under a fresh
    /// connection id.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns, which is
    /// where an endpoint the catalog rejects — unknown port, wrong direction,
    /// mismatched schema type, or a single-connection port already taken —
    /// surfaces as [`EditorGraphUiAdapterError::InvalidCommandBatch`].
    pub fn plan_connect_ports(
        &self,
        client_batch_id: impl Into<String>,
        from: GraphPortRef,
        to: GraphPortRef,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::Connect {
                connection: GraphConnection::new(GraphConnectionId::new_v7(), from, to),
            }],
        )
    }

    /// Plans a single-command batch that sets, or clears with `None`, the
    /// literal input value on one port.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns.
    pub fn plan_set_input_value(
        &self,
        client_batch_id: impl Into<String>,
        node_id: GraphNodeId,
        port_id: NodePortId,
        value: Option<ReflectedValueEnvelope>,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::SetInputValue {
                node_id,
                port_id,
                value,
            }],
        )
    }

    /// Plans a single-command batch that replaces one connection's route.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns.
    pub fn plan_connection_route(
        &self,
        client_batch_id: impl Into<String>,
        connection_id: GraphConnectionId,
        route: GraphConnectionRoute,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::SetConnectionRoute {
                connection_id,
                route,
            }],
        )
    }

    /// Plans a route replacement that moves one waypoint of a connection to
    /// `position`, leaving the rest of the route as authored.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::UnknownConnection`] if
    /// `connection_id` is not in the projected document,
    /// [`EditorGraphUiAdapterError::UnknownRouteAnchor`] if that connection's
    /// route carries no anchor `anchor_id`, or any error
    /// [`Self::plan_connection_route`] returns.
    pub fn plan_move_route_anchor(
        &self,
        client_batch_id: impl Into<String>,
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
        position: GraphPoint,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        let connection = self
            .snapshot
            .document
            .connections
            .iter()
            .find(|connection| connection.id == connection_id)
            .ok_or(EditorGraphUiAdapterError::UnknownConnection { connection_id })?;
        let mut route = connection.route.clone();
        let anchor = route
            .anchors
            .iter_mut()
            .find(|anchor| anchor.id == anchor_id)
            .ok_or(EditorGraphUiAdapterError::UnknownRouteAnchor {
                connection_id,
                anchor_id,
            })?;
        anchor.position = position;
        self.plan_connection_route(client_batch_id, connection_id, route)
    }

    /// Plans a single-command batch that upserts `comment` into the document.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns.
    pub fn plan_create_comment(
        &self,
        client_batch_id: impl Into<String>,
        comment: GraphComment,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::UpsertComment { comment }],
        )
    }

    /// Plans a comment upsert that only changes the comment's bounds.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::UnknownComment`] if `comment_id` is
    /// not in the projected document, or any error
    /// [`Self::plan_required_commands`] returns.
    pub fn plan_move_comment(
        &self,
        client_batch_id: impl Into<String>,
        comment_id: GraphCommentId,
        bounds: GraphCommentBounds,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        let mut comment = self
            .snapshot
            .document
            .comments
            .iter()
            .find(|comment| comment.id == comment_id)
            .cloned()
            .ok_or(EditorGraphUiAdapterError::UnknownComment { comment_id })?;
        comment.bounds = bounds;
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::UpsertComment { comment }],
        )
    }

    /// Plans a comment upsert that only changes the comment's text.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::UnknownComment`] if `comment_id` is
    /// not in the projected document, or any error
    /// [`Self::plan_required_commands`] returns.
    pub fn plan_set_comment_text(
        &self,
        client_batch_id: impl Into<String>,
        comment_id: GraphCommentId,
        text: String,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        let mut comment = self
            .snapshot
            .document
            .comments
            .iter()
            .find(|comment| comment.id == comment_id)
            .cloned()
            .ok_or(EditorGraphUiAdapterError::UnknownComment { comment_id })?;
        comment.text = text;
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::UpsertComment { comment }],
        )
    }

    /// Plans a single-command batch that removes one comment.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns.
    pub fn plan_remove_comment(
        &self,
        client_batch_id: impl Into<String>,
        comment_id: GraphCommentId,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        self.plan_required_commands(
            client_batch_id,
            vec![GraphCommand::RemoveComment { comment_id }],
        )
    }

    /// Runs `solver` over the current document and geometry and plans the
    /// commands it produced, or `None` when the layout is already settled.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::Layout`] wrapping whatever
    /// [`GraphLayoutSolver::solve`] rejected, or any error
    /// [`Self::plan_commands`] returns for the solved commands.
    pub fn plan_layout<S: GraphLayoutSolver>(
        &self,
        solver: &S,
        operation: GraphLayoutOperation,
        client_batch_id: impl Into<String>,
    ) -> EditorGraphUiResult<Option<GraphCommandBatchSnapshot>> {
        let result = solver.solve(GraphLayoutRequest::new(
            &self.snapshot.document,
            &self.catalog,
            &self.geometry,
            operation,
        ))?;
        self.plan_commands(client_batch_id, result.commands)
    }

    /// Plans `commands` as one batch, or `None` when there is nothing to send.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::plan_required_commands`] returns for a
    /// non-empty `commands`.
    pub fn plan_commands(
        &self,
        client_batch_id: impl Into<String>,
        commands: Vec<GraphCommand>,
    ) -> EditorGraphUiResult<Option<GraphCommandBatchSnapshot>> {
        if commands.is_empty() {
            return Ok(None);
        }
        Ok(Some(
            self.plan_required_commands(client_batch_id, commands)?,
        ))
    }

    /// Dry-runs `commands` against a clone of the projected document and, if
    /// they apply, returns the batch to send at the adapter's revision.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::EmptyClientBatchId`] if
    /// `client_batch_id` trims to nothing, or
    /// [`EditorGraphUiAdapterError::InvalidCommandBatch`] if the local dry run
    /// rejects the commands against the node catalog. Nothing is planned unless
    /// every command applies, so a rejected batch never reaches project-host.
    pub fn plan_required_commands(
        &self,
        client_batch_id: impl Into<String>,
        commands: Vec<GraphCommand>,
    ) -> EditorGraphUiResult<GraphCommandBatchSnapshot> {
        let client_batch_id = client_batch_id.into();
        if client_batch_id.trim().is_empty() {
            return Err(EditorGraphUiAdapterError::EmptyClientBatchId);
        }
        let mut projected = self.snapshot.document.clone();
        projected.apply_commands(commands.clone(), &self.catalog)?;
        Ok(GraphCommandBatchSnapshot {
            document_id: self.snapshot.document_id.clone(),
            expected_revision: Some(self.snapshot.revision),
            client_batch_id,
            commands,
        })
    }

    /// Folds project-host's verdict on `batch` into the projection, returning
    /// whether the batch was accepted and applied locally.
    ///
    /// # Errors
    ///
    /// Returns [`EditorGraphUiAdapterError::StatusMismatch`] if `status` does
    /// not belong to `batch` and this adapter — a different document id, a
    /// different client batch id, an expected revision that is no longer the
    /// adapter's revision, an accepted status whose applied-command count is not
    /// the batch length, or a rejected status claiming applied commands, since
    /// graph edits are transactional. Returns
    /// [`EditorGraphUiAdapterError::InvalidCommandBatch`] if replaying an
    /// accepted batch against the projected document fails.
    pub fn apply_project_host_status(
        &mut self,
        batch: &GraphCommandBatchSnapshot,
        status: &GraphCommandStatusSnapshot,
    ) -> EditorGraphUiResult<bool> {
        self.validate_status_matches_batch(batch, status)?;
        match &status.outcome {
            GraphCommandStatusOutcome::Rejected { .. } => Ok(false),
            GraphCommandStatusOutcome::Accepted { revision } => {
                let mut document = self.snapshot.document.clone();
                document.apply_commands(batch.commands.clone(), &self.catalog)?;
                self.snapshot.document = document;
                self.snapshot.revision = *revision;
                self.rebuild_spatial_index();
                Ok(true)
            }
        }
    }

    fn validate_status_matches_batch(
        &self,
        batch: &GraphCommandBatchSnapshot,
        status: &GraphCommandStatusSnapshot,
    ) -> EditorGraphUiResult<()> {
        if batch.document_id != self.snapshot.document_id {
            return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                "batch document `{}` does not match adapter document `{}`",
                batch.document_id.as_str(),
                self.snapshot.document_id.as_str()
            )));
        }
        if status.document_id != batch.document_id {
            return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                "status document `{}` does not match batch document `{}`",
                status.document_id.as_str(),
                batch.document_id.as_str()
            )));
        }
        if status.client_batch_id != batch.client_batch_id {
            return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                "status batch id `{}` does not match client batch id `{}`",
                status.client_batch_id, batch.client_batch_id
            )));
        }
        if batch.expected_revision != Some(self.snapshot.revision) {
            return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                "batch expected revision {:?} does not match adapter revision {}",
                batch.expected_revision.map(|revision| revision.0),
                self.snapshot.revision.0
            )));
        }

        match &status.outcome {
            GraphCommandStatusOutcome::Accepted { .. } => {
                if status.applied_command_count as usize != batch.commands.len() {
                    return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                        "accepted status applied {} commands but batch has {} commands",
                        status.applied_command_count,
                        batch.commands.len()
                    )));
                }
            }
            GraphCommandStatusOutcome::Rejected { .. } => {
                if status.applied_command_count != 0 {
                    return Err(EditorGraphUiAdapterError::StatusMismatch(format!(
                        "rejected status reported {} applied commands; graph edits are transactional",
                        status.applied_command_count
                    )));
                }
            }
        }
        Ok(())
    }

    fn rebuild_spatial_index(&mut self) {
        self.spatial_index = GraphSpatialIndex::build(
            &self.snapshot.document,
            &self.catalog,
            &self.geometry,
            GraphLayoutTuning::default(),
        );
    }

    #[must_use]
    pub fn to_ui_projection(
        &self,
        saved_revision: Option<DocumentRevision>,
        graph_type: Option<&GraphTypeDescriptor>,
    ) -> EditorGraphDocumentProjection {
        EditorGraphDocumentProjection::document(graph_document_projection_from_snapshot(
            &self.snapshot,
            &self.catalog,
            &self.geometry,
            saved_revision,
            graph_type,
        ))
    }
}

/// Projects a validated graph snapshot into the panel's render data, resolving
/// node bounds, port anchors, and connection route points from `geometry`.
///
/// # Panics
///
/// Panics if `snapshot` was not validated against `catalog` first: resolving a
/// connection's route points expects both endpoints to exist in the node
/// catalog, and the per-node and per-port projections likewise expect the node
/// type version and port descriptor to be present. Callers reach this through
/// [`EditorGraphUiAdapter`], which validates on construction and on every
/// snapshot replacement.
#[must_use]
pub fn graph_document_projection_from_snapshot(
    snapshot: &GraphDocumentSnapshot,
    catalog: &NodeTypeCatalog,
    geometry: &GraphGeometrySnapshot,
    saved_revision: Option<DocumentRevision>,
    graph_type: Option<&GraphTypeDescriptor>,
) -> GraphDocumentProjectionData {
    let tuning = GraphLayoutTuning::default();
    GraphDocumentProjectionData {
        document_id: snapshot.document_id.as_str().to_string(),
        graph_type: snapshot.document.graph_type.clone(),
        graph_type_info: graph_type.map(graph_type_creation_projection_from_descriptor),
        revision: snapshot.revision.0,
        saved_revision: saved_revision.map(|revision| revision.0),
        unsaved_changes: saved_revision != Some(snapshot.revision),
        catalog_version: catalog.catalog_version,
        nodes: snapshot
            .document
            .nodes
            .iter()
            .map(|node| graph_node_projection(node, snapshot, catalog, geometry, tuning))
            .collect(),
        connections: snapshot
            .document
            .connections
            .iter()
            .map(|connection| {
                let route_points = graph_connection_route_points(
                    &snapshot.document,
                    catalog,
                    geometry,
                    tuning,
                    connection,
                )
                .expect("validated graph connection endpoints exist in node catalog");
                let mut route_anchors = Vec::new();
                if let Some(point) = route_points.first().copied() {
                    route_anchors.push(GraphRouteAnchorProjectionData {
                        anchor_id: format!("{}:from", connection.id),
                        kind: GraphRouteAnchorKindData::PortEndpoint,
                        x: point.x,
                        y: point.y,
                    });
                }
                route_anchors.extend(connection.route.anchors.iter().map(|anchor| {
                    GraphRouteAnchorProjectionData {
                        anchor_id: anchor.id.to_string(),
                        kind: graph_route_anchor_kind_projection(anchor.kind),
                        x: anchor.position.x,
                        y: anchor.position.y,
                    }
                }));
                if let Some(point) = route_points.last().copied() {
                    route_anchors.push(GraphRouteAnchorProjectionData {
                        anchor_id: format!("{}:to", connection.id),
                        kind: GraphRouteAnchorKindData::PortEndpoint,
                        x: point.x,
                        y: point.y,
                    });
                }

                GraphConnectionProjectionData {
                    connection_id: connection.id.to_string(),
                    from_node_id: connection.from.node_id.to_string(),
                    to_node_id: connection.to.node_id.to_string(),
                    points: route_points
                        .into_iter()
                        .map(graph_point_projection)
                        .collect(),
                    route_anchors,
                    selected: false,
                }
            })
            .collect(),
        comments: snapshot
            .document
            .comments
            .iter()
            .map(graph_comment_projection)
            .collect(),
        diagnostics: Vec::new(),
    }
}

fn graph_comment_projection(comment: &GraphComment) -> GraphCommentProjectionData {
    GraphCommentProjectionData {
        comment_id: comment.id.to_string(),
        text: comment.text.clone(),
        x: comment.bounds.x,
        y: comment.bounds.y,
        width: comment.bounds.width,
        height: comment.bounds.height,
        selected: false,
    }
}

fn graph_node_projection(
    node: &GraphNode,
    snapshot: &GraphDocumentSnapshot,
    catalog: &NodeTypeCatalog,
    geometry: &GraphGeometrySnapshot,
    tuning: GraphLayoutTuning,
) -> GraphNodeProjectionData {
    let descriptor = catalog
        .node_type_version(&node.node_type, node.node_type_version)
        .expect("validated graph node type exists in node catalog");
    let bounds = graph_node_bounds(node, geometry, tuning);
    GraphNodeProjectionData {
        node_id: node.id.to_string(),
        node_type: node.node_type.as_str().to_string(),
        label: descriptor.display_name.clone(),
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
        selected: false,
        source_links: descriptor
            .source_links
            .iter()
            .map(graph_node_source_link_projection)
            .collect(),
        ports: descriptor
            .ports
            .iter()
            .map(|port| {
                graph_port_projection(node, port, snapshot, catalog, geometry, tuning, bounds)
            })
            .collect(),
    }
}

fn graph_node_source_link_projection(link: &NodeSourceLink) -> GraphNodeSourceLinkProjectionData {
    GraphNodeSourceLinkProjectionData {
        package: link.package.clone(),
        module_path: link.module_path.clone(),
        symbol_path: link.symbol_path.clone(),
        file: link.file.clone(),
        line: link.line,
        column: link.column,
        docs_url: link.docs_url.clone(),
    }
}

fn graph_build_status_from_save_result(
    result: &SaveGraphDocumentResult,
) -> GraphBuildStatusProjectionData {
    debug_assert_eq!(
        result.asset_record.asset_guid, result.asset_record.entry.asset_guid,
        "session-supervisor SaveGraphDocumentResult must echo the same asset GUID at the record and entry levels"
    );
    graph_build_status_from_asset_entry(&result.saved.document_id, &result.asset_record.entry)
}

fn graph_build_status_from_asset_entry(
    document_id: &DocumentId,
    entry: &WorkspaceEntry,
) -> GraphBuildStatusProjectionData {
    GraphBuildStatusProjectionData {
        document_id: document_id.as_str().to_string(),
        source_path: entry.source_path.clone(),
        asset_guid: entry.asset_guid.to_string(),
        source_status: graph_build_source_status(entry.diff),
        entry_id: entry.entry_id,
        content_hash: entry.content_hash.clone(),
        latest_job: entry.jobs.last().map(graph_build_job_projection),
    }
}

fn graph_build_job_projection(
    activity: &az_proto_asset::JobActivity,
) -> GraphBuildJobProjectionData {
    let attempt = activity.attempt.as_ref();
    GraphBuildJobProjectionData {
        job_id: activity.job.job_id,
        attempt_id: attempt.map(|attempt| attempt.attempt_id),
        job_key: activity.job.key.clone(),
        platform: activity.job.platform.clone(),
        ordinal: attempt.map(|attempt| attempt.ordinal),
        status: attempt.map_or_else(
            || graph_build_job_status(activity.job.status),
            |attempt| graph_build_attempt_status(attempt.status),
        ),
        error_count: attempt.map_or(0, |attempt| attempt.error_count),
        warning_count: attempt.map_or(0, |attempt| attempt.warning_count),
    }
}

const fn graph_build_source_status(
    status: az_proto_asset::WorkspaceEntryDiff,
) -> GraphBuildSourceStatusData {
    match status {
        az_proto_asset::WorkspaceEntryDiff::Clean => GraphBuildSourceStatusData::Clean,
        az_proto_asset::WorkspaceEntryDiff::Added => GraphBuildSourceStatusData::Added,
        az_proto_asset::WorkspaceEntryDiff::Modified => GraphBuildSourceStatusData::Modified,
        az_proto_asset::WorkspaceEntryDiff::Deleted => GraphBuildSourceStatusData::Deleted,
        az_proto_asset::WorkspaceEntryDiff::Conflicted => GraphBuildSourceStatusData::Conflicted,
    }
}

const fn graph_build_job_status(status: az_proto_asset::JobStatus) -> GraphBuildJobStatusData {
    match status {
        az_proto_asset::JobStatus::Queued => GraphBuildJobStatusData::Queued,
        az_proto_asset::JobStatus::Leased => GraphBuildJobStatusData::Leased,
        az_proto_asset::JobStatus::Succeeded => GraphBuildJobStatusData::Succeeded,
        az_proto_asset::JobStatus::Failed => GraphBuildJobStatusData::Failed,
    }
}

const fn graph_build_attempt_status(
    status: az_proto_asset::AttemptStatus,
) -> GraphBuildJobStatusData {
    match status {
        az_proto_asset::AttemptStatus::Leased => GraphBuildJobStatusData::Leased,
        az_proto_asset::AttemptStatus::Succeeded => GraphBuildJobStatusData::Succeeded,
        az_proto_asset::AttemptStatus::Failed => GraphBuildJobStatusData::Failed,
        az_proto_asset::AttemptStatus::Abandoned => GraphBuildJobStatusData::Abandoned,
    }
}

fn graph_build_console_line(status: &GraphBuildStatusProjectionData) -> String {
    status.latest_job.as_ref().map_or_else(
        || {
            format!(
                "graph build status: {} ({}) with no matching asset-builder job",
                status.source_path,
                status.source_status.label()
            )
        },
        |job| {
            format!(
                "graph build status: {} -> {}:{} #{} {}",
                status.source_path,
                job.job_key,
                job.platform,
                job.ordinal
                    .map_or_else(|| "-".to_owned(), |ordinal| ordinal.to_string()),
                job.status.label()
            )
        },
    )
}

fn graph_port_projection(
    node: &GraphNode,
    port: &NodePortDescriptor,
    snapshot: &GraphDocumentSnapshot,
    catalog: &NodeTypeCatalog,
    geometry: &GraphGeometrySnapshot,
    tuning: GraphLayoutTuning,
    bounds: GraphRect,
) -> GraphPortProjectionData {
    let anchor = graph_port_anchor(
        &snapshot.document,
        catalog,
        geometry,
        tuning,
        &GraphPortRef::new(node.id, port.id),
    )
    .expect("validated graph port exists in node catalog");
    GraphPortProjectionData {
        port_id: port.id.0,
        name: port.name.clone(),
        direction: graph_port_direction_projection(port.direction),
        side: graph_port_side_projection(port.layout.side),
        value: graph_input_value_projection(node, port),
        x: anchor.x - bounds.x,
        y: anchor.y - bounds.y,
    }
}

fn graph_input_value_projection(
    node: &GraphNode,
    port: &NodePortDescriptor,
) -> Option<GraphInputValueProjectionData> {
    if port.direction != NodePortDirection::Input {
        return None;
    }
    let schema_type = match &port.value {
        NodePortValue::Data { schema_type } => schema_type.clone(),
        NodePortValue::DynamicData { group, .. } => format!("dynamic:{group}"),
        NodePortValue::Execution => return None,
    };
    Some(GraphInputValueProjectionData {
        schema_type,
        current_value: node.input_values.get(&port.id).cloned(),
        default_value: port.default_value.clone(),
    })
}

const fn graph_point_projection(point: GraphPoint) -> GraphPointProjectionData {
    GraphPointProjectionData::new(point.x, point.y)
}

const fn graph_port_direction_projection(direction: NodePortDirection) -> GraphPortDirectionData {
    match direction {
        NodePortDirection::Input => GraphPortDirectionData::Input,
        NodePortDirection::Output => GraphPortDirectionData::Output,
    }
}

const fn graph_port_side_projection(side: NodePortSide) -> GraphPortSideData {
    match side {
        NodePortSide::North => GraphPortSideData::North,
        NodePortSide::East => GraphPortSideData::East,
        NodePortSide::South => GraphPortSideData::South,
        NodePortSide::West => GraphPortSideData::West,
    }
}

const fn graph_route_anchor_kind_projection(
    kind: GraphRouteAnchorKind,
) -> GraphRouteAnchorKindData {
    match kind {
        GraphRouteAnchorKind::UserWaypoint => GraphRouteAnchorKindData::UserWaypoint,
        GraphRouteAnchorKind::SolverWaypoint => GraphRouteAnchorKindData::SolverWaypoint,
        GraphRouteAnchorKind::Junction => GraphRouteAnchorKindData::Junction,
    }
}

#[derive(Clone)]
pub struct EditorGraphController {
    #[cfg(test)]
    client: ProjectHostClient,
    project_host_descriptor: Option<ServiceDescriptor>,
    supervisor_descriptor: Option<ServiceDescriptor>,
    #[cfg(test)]
    supervisor: Option<SessionSupervisorClient>,
    session_slug: Option<String>,
    session_id: Option<Uuid>,
    side_channel_root: PathBuf,
    node_catalog: NodeTypeCatalog,
    graph_catalog: GraphTypeCatalog,
    adapter: Option<EditorGraphUiAdapter>,
    current_document_id: Option<DocumentId>,
    saved_revision: Option<DocumentRevision>,
    build_status: Option<GraphBuildStatusProjectionData>,
    graph_document_entries: Vec<GraphDocumentEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GraphDocumentEntry {
    document_id: DocumentId,
    source_path: String,
    revision: DocumentRevision,
    saved_revision: Option<DocumentRevision>,
    unsaved_changes: bool,
    loaded: bool,
}

impl EditorGraphController {
    /// Dials the attached session's project host and loads the node and graph
    /// type catalogs the panel projects against. No document is selected yet.
    ///
    /// # Errors
    ///
    /// Returns any error [`ProjectHostClient::connect_for_session`],
    /// [`ProjectHostClient::load_node_type_catalog`], or
    /// [`ProjectHostClient::load_graph_type_catalog`] returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn connect_attached(session: &EditorAttachSession) -> EditorResult<Self> {
        let client = ProjectHostClient::connect_for_session(
            &session.services.project_host,
            session.session_id,
        )
        .await?;
        let node_catalog = client.load_node_type_catalog().await?;
        let graph_catalog = client.load_graph_type_catalog().await?;
        Ok(Self {
            #[cfg(test)]
            client,
            project_host_descriptor: Some(session.services.project_host.clone()),
            supervisor_descriptor: Some(session.session_supervisor.clone()),
            #[cfg(test)]
            supervisor: None,
            session_slug: Some(session.session_slug.clone()),
            session_id: Some(session.session_id),
            side_channel_root: session.run_dir.join("editor").join("graph-command-batches"),
            node_catalog,
            graph_catalog,
            adapter: None,
            current_document_id: None,
            saved_revision: None,
            build_status: None,
            graph_document_entries: Vec::new(),
        })
    }

    #[cfg(test)]
    #[must_use]
    pub fn new_for_tests(
        client: ProjectHostClient,
        node_catalog: NodeTypeCatalog,
        graph_catalog: GraphTypeCatalog,
        side_channel_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            #[cfg(test)]
            client,
            project_host_descriptor: None,
            supervisor_descriptor: None,
            #[cfg(test)]
            supervisor: None,
            session_slug: None,
            session_id: None,
            side_channel_root: side_channel_root.into(),
            node_catalog,
            graph_catalog,
            adapter: None,
            current_document_id: None,
            saved_revision: None,
            build_status: None,
            graph_document_entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn current_projection(&self) -> EditorGraphDocumentProjection {
        let creation_catalog =
            graph_creation_catalog_projection_from_graph_type_catalog(&self.graph_catalog);
        let current_graph_type = self.adapter.as_ref().and_then(|adapter| {
            self.graph_type_for_document(&adapter.snapshot.document.graph_type)
        });
        let node_palette =
            node_palette_projection_from_node_type_catalog(&self.node_catalog, current_graph_type);
        let graph_documents = graph_document_list_projection_from_entries(
            &self.graph_document_entries,
            &self.graph_catalog,
            self.current_document_id.as_ref(),
        );
        self.adapter
            .as_ref()
            .map_or_else(EditorGraphDocumentProjection::empty, |adapter| {
                adapter.to_ui_projection(self.saved_revision, current_graph_type)
            })
            .with_build_status(self.build_status.clone())
            .with_graph_documents(graph_documents)
            .with_creation_catalog(creation_catalog)
            .with_node_palette(node_palette)
    }

    /// Creates a new document of `graph_type` at the name's derived native path
    /// and selects it.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::GraphTypeNotPublished`] if `graph_type` is absent
    /// from the loaded graph type catalog, [`EditorError::ServiceDiscovery`] if
    /// the project-host client cannot be resolved for the attached session, any
    /// error [`ProjectHostClient::create_graph_document_from_creation_data`]
    /// returns — which covers the document-name and extension rejections from
    /// [`graph_document_id_from_creation_data`] — any error from reloading the
    /// document list, and [`EditorError::GraphUiAdapter`] if the created
    /// snapshot does not validate against the node catalog.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn create_graph_document(
        mut self,
        graph_type: String,
        document_name: String,
    ) -> EditorResult<Self> {
        let creation_catalog = graph_creation_catalog_from_graph_type_catalog(&self.graph_catalog);
        let graph_type = creation_catalog
            .graph_type(&graph_type)
            .cloned()
            .ok_or_else(|| EditorError::GraphTypeNotPublished { graph_type })?;
        let client = self.project_host_client("graph document create").await?;
        let snapshot = client
            .create_graph_document_from_creation_data(&graph_type, &document_name)
            .await?;
        let document_id = snapshot.document_id.clone();
        let revision = snapshot.revision;
        self.graph_document_entries = self.load_graph_document_entries().await?;
        let saved_revision = None;
        self.upsert_graph_document_entry(document_id.clone(), revision, saved_revision, true, true);
        let adapter = EditorGraphUiAdapter::new(
            snapshot,
            self.node_catalog.clone(),
            GraphGeometrySnapshot::default(),
        )?;
        self.current_document_id = Some(document_id);
        self.saved_revision = saved_revision;
        self.build_status = None;
        self.adapter = Some(adapter);
        Ok(self)
    }

    /// Reloads the document list and re-reads the selected document, falling
    /// back to the first graph document when nothing is selected.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::load_graph_document`] returns, plus the
    /// asset-processor discovery and paging errors raised while reloading the
    /// document list. Finding no graph document at all is not an error: the
    /// controller clears its selection and returns.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn refresh_current_or_first(mut self) -> EditorResult<Self> {
        self.graph_document_entries = self.load_graph_document_entries().await?;
        let document_id = self
            .current_document_id
            .clone()
            .or_else(|| self.first_graph_document_id());
        let Some(document_id) = document_id else {
            self.adapter = None;
            self.current_document_id = None;
            self.saved_revision = None;
            self.build_status = None;
            return Ok(self);
        };
        self.load_graph_document(document_id).await
    }

    /// Reads `document_id` from project-host and makes it the selection.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if the project-host client
    /// cannot be resolved for the attached session, any error
    /// [`ProjectHostClient::graph_document_snapshot`] returns for an unknown or
    /// unreadable document, the asset-processor discovery and paging errors
    /// raised while reloading the document list, and
    /// [`EditorError::GraphUiAdapter`] if the loaded snapshot does not validate
    /// against the node catalog.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn load_graph_document(mut self, document_id: DocumentId) -> EditorResult<Self> {
        let client = self.project_host_client("graph document load").await?;
        let snapshot = client.graph_document_snapshot(&document_id).await?;
        self.graph_document_entries = self.load_graph_document_entries().await?;
        let saved_revision = self
            .document_list_entry_for(&document_id)
            .and_then(|entry| entry.saved_revision)
            .or(Some(snapshot.revision));
        self.upsert_graph_document_entry(
            document_id.clone(),
            snapshot.revision,
            saved_revision,
            false,
            true,
        );
        let adapter = EditorGraphUiAdapter::new(
            snapshot,
            self.node_catalog.clone(),
            GraphGeometrySnapshot::default(),
        )?;
        self.current_document_id = Some(document_id);
        self.saved_revision = saved_revision;
        self.build_status = None;
        self.adapter = Some(adapter);
        Ok(self)
    }

    /// Solves an automatic layout for the selected document and sends the
    /// resulting move commands as one batch.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if
    /// [`DefaultGraphLayoutSolver`] fails or the solved commands do not plan,
    /// and any error the batch apply raises: project-host discovery, the
    /// [`ProjectHostClient::apply_graph_commands`] round trip, or a status
    /// project-host returns that does not match the batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn auto_layout_current(self) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            let solver = DefaultGraphLayoutSolver::default();
            adapter.plan_layout(
                &solver,
                GraphLayoutOperation::AutoLayout(GraphAutoLayoutOptions::default()),
                fresh_graph_batch_id("auto-layout"),
            )?
        };
        self.apply_planned_batch(batch).await
    }

    /// Re-routes every connection in the selected document and sends the
    /// resulting route commands as one batch.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if
    /// [`DefaultGraphLayoutSolver`] fails or the solved commands do not plan,
    /// and any error the batch apply raises: project-host discovery, the
    /// [`ProjectHostClient::apply_graph_commands`] round trip, or a status
    /// project-host returns that does not match the batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn route_current_connections(self) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            let solver = DefaultGraphLayoutSolver::default();
            adapter.plan_layout(
                &solver,
                GraphLayoutOperation::RouteConnections(GraphRouteOptions::default()),
                fresh_graph_batch_id("route"),
            )?
        };
        self.apply_planned_batch(batch).await
    }

    /// Adds a node of `node_type` at `layout`, seeded with the descriptor's
    /// default input values.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::GraphNodeTypeNotPublished`] if the node catalog
    /// has no `node_type` at `node_type_version`,
    /// [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the add does not plan
    /// against the projected document, and any error the batch apply raises:
    /// project-host discovery, the [`ProjectHostClient::apply_graph_commands`]
    /// round trip, or a status project-host returns that does not match the
    /// batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn add_node_to_current(
        self,
        node_type: String,
        node_type_version: u32,
        layout: GraphNodeLayout,
    ) -> EditorResult<Self> {
        let descriptor = self
            .node_catalog
            .node_type_version(&NodeTypeId::new(node_type.clone()), node_type_version)
            .cloned()
            .ok_or(EditorError::GraphNodeTypeNotPublished {
                node_type,
                version: node_type_version,
            })?;
        let node = graph_node_from_descriptor_for_ui(&descriptor, layout);
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_required_commands(
                fresh_graph_batch_id("add-node"),
                vec![GraphCommand::AddNode { node }],
            )?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Moves one node of the selected document to `layout`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the move does not plan
    /// against the projected document — `node_id` is not in it — and any error
    /// the batch apply raises: project-host discovery, the
    /// [`ProjectHostClient::apply_graph_commands`] round trip, or a status
    /// project-host returns that does not match the batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn move_node_in_current(
        self,
        node_id: GraphNodeId,
        layout: GraphNodeLayout,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_move_node(fresh_graph_batch_id("move-node"), node_id, layout)?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Removes one node, and the edges touching it, from the selected document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the removal does not plan
    /// against the projected document — `node_id` is not in it — and any error
    /// the batch apply raises: project-host discovery, the
    /// [`ProjectHostClient::apply_graph_commands`] round trip, or a status
    /// project-host returns that does not match the batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn remove_node_from_current(self, node_id: GraphNodeId) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_remove_node(fresh_graph_batch_id("remove-node"), node_id)?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Drags one route waypoint of a connection to `position`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] wrapping
    /// [`EditorGraphUiAdapterError::UnknownConnection`] or
    /// [`EditorGraphUiAdapterError::UnknownRouteAnchor`] when `connection_id` or
    /// `anchor_id` is absent from the projected document, and any error the
    /// batch apply raises: project-host discovery, the
    /// [`ProjectHostClient::apply_graph_commands`] round trip, or a status
    /// project-host returns that does not match the batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn move_route_anchor_in_current(
        self,
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
        position: GraphPoint,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_move_route_anchor(
                fresh_graph_batch_id("move-route-anchor"),
                connection_id,
                anchor_id,
                position,
            )?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Adds a comment with a fresh id at `bounds` to the selected document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the upsert does not plan
    /// against the projected document, and any error the batch apply raises:
    /// project-host discovery, the [`ProjectHostClient::apply_graph_commands`]
    /// round trip, or a status project-host returns that does not match the
    /// batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn create_comment_in_current(
        self,
        text: String,
        bounds: GraphCommentBounds,
    ) -> EditorResult<Self> {
        let comment = GraphComment {
            id: GraphCommentId::new_v7(),
            text,
            bounds,
        };
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_create_comment(fresh_graph_batch_id("create-comment"), comment)?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Moves one comment of the selected document to `bounds`.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] wrapping
    /// [`EditorGraphUiAdapterError::UnknownComment`] if `comment_id` is absent
    /// from the projected document, and any error the batch apply raises:
    /// project-host discovery, the [`ProjectHostClient::apply_graph_commands`]
    /// round trip, or a status project-host returns that does not match the
    /// batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn move_comment_in_current(
        self,
        comment_id: GraphCommentId,
        bounds: GraphCommentBounds,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_move_comment(fresh_graph_batch_id("move-comment"), comment_id, bounds)?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Replaces one comment's text in the selected document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] wrapping
    /// [`EditorGraphUiAdapterError::UnknownComment`] if `comment_id` is absent
    /// from the projected document, and any error the batch apply raises:
    /// project-host discovery, the [`ProjectHostClient::apply_graph_commands`]
    /// round trip, or a status project-host returns that does not match the
    /// batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn set_comment_text_in_current(
        self,
        comment_id: GraphCommentId,
        text: String,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_set_comment_text(
                fresh_graph_batch_id("set-comment-text"),
                comment_id,
                text,
            )?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Removes one comment from the selected document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the removal does not plan
    /// against the projected document, and any error the batch apply raises:
    /// project-host discovery, the [`ProjectHostClient::apply_graph_commands`]
    /// round trip, or a status project-host returns that does not match the
    /// batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn remove_comment_from_current(
        self,
        comment_id: GraphCommentId,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_remove_comment(fresh_graph_batch_id("remove-comment"), comment_id)?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Connects `from` to `to` in the selected document.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the connection does not
    /// plan against the projected document — an unknown endpoint, a direction
    /// or schema-type mismatch, or a single-connection port already taken — and
    /// any error the batch apply raises: project-host discovery, the
    /// [`ProjectHostClient::apply_graph_commands`] round trip, or a status
    /// project-host returns that does not match the batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn connect_ports_in_current(
        self,
        from: GraphPortRef,
        to: GraphPortRef,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_connect_ports(fresh_graph_batch_id("connect-ports"), from, to)?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Sets, or clears with `None`, one node's literal input value.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::GraphUiAdapter`] if the assignment does not
    /// plan against the projected document — an unknown node or port, or a port
    /// that is not an input data port — and any error the batch apply raises:
    /// project-host discovery, the [`ProjectHostClient::apply_graph_commands`]
    /// round trip, or a status project-host returns that does not match the
    /// batch.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn set_input_value_in_current(
        self,
        node_id: GraphNodeId,
        port_id: NodePortId,
        value: Option<ReflectedValueEnvelope>,
    ) -> EditorResult<Self> {
        let batch = {
            let adapter = self
                .adapter
                .as_ref()
                .ok_or(EditorError::MissingGraphDocumentSelection)?;
            adapter.plan_set_input_value(
                fresh_graph_batch_id("set-input-value"),
                node_id,
                port_id,
                value,
            )?
        };
        self.apply_planned_batch(Some(batch)).await
    }

    /// Persists the selected document, preferring the session supervisor so the
    /// save also reports asset-builder status, and falling back to a
    /// project-host record save when no supervisor is attached.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::ServiceDiscovery`] if the attached session
    /// carries no session slug or session id where one is required, any error
    /// [`SessionSupervisorClient::save_graph_document`] or
    /// [`ProjectHostClient::save_graph_document_record`] returns on the path
    /// taken, and the asset-processor discovery and paging errors raised while
    /// reloading the document list.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn save_current(mut self) -> EditorResult<Self> {
        let document_id = self
            .current_document_id
            .clone()
            .ok_or(EditorError::MissingGraphDocumentSelection)?;
        let (revision, build_status) = if let Some(supervisor) = self
            .optional_session_supervisor_client("graph document save", true)
            .await?
        {
            let session_slug = self.attached_session_slug("graph document save")?;
            let result = supervisor
                .save_graph_document(session_slug, &document_id)
                .await?;
            (
                result.saved.revision,
                Some(graph_build_status_from_save_result(&result)),
            )
        } else {
            let client = self.project_host_client("graph document save").await?;
            (
                client
                    .save_graph_document_record(&document_id)
                    .await?
                    .revision,
                None,
            )
        };
        self.saved_revision = Some(revision);
        self.build_status = build_status;
        self.graph_document_entries = self.load_graph_document_entries().await?;
        self.upsert_graph_document_entry(document_id, revision, Some(revision), false, true);
        Ok(self)
    }

    /// Saves the selected document through the session supervisor, which is
    /// what queues the asset-builder job, and adopts the returned build status.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, [`EditorError::ServiceDiscovery`] if the controller holds no
    /// session-supervisor descriptor, session id, or session slug — unlike
    /// [`Self::save_current`] there is no project-host fallback — any error
    /// [`SessionSupervisorClient::save_graph_document`] returns, and the
    /// asset-processor discovery and paging errors raised while reloading the
    /// document list.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn build_current(mut self) -> EditorResult<Self> {
        let document_id = self
            .current_document_id
            .clone()
            .ok_or(EditorError::MissingGraphDocumentSelection)?;
        let supervisor = self
            .session_supervisor_client("graph document build")
            .await?;
        let session_slug = self.attached_session_slug("graph document build")?;
        let result = supervisor
            .save_graph_document(session_slug, &document_id)
            .await?;
        self.saved_revision = Some(result.saved.revision);
        self.build_status = Some(graph_build_status_from_save_result(&result));
        self.graph_document_entries = self.load_graph_document_entries().await?;
        self.upsert_graph_document_entry(
            document_id,
            result.saved.revision,
            Some(result.saved.revision),
            false,
            true,
        );
        Ok(self)
    }

    /// Re-reads the asset-processor workspace entry for the selected document's
    /// source path and adopts its latest builder job as the build status.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::MissingGraphDocumentSelection`] if no document is
    /// selected, and [`EditorError::ServiceDiscovery`] if the document id trims
    /// to an empty source path, if the controller holds no session slug,
    /// session id, or session-supervisor descriptor, if the asset-processor
    /// descriptor the supervisor resolves does not match the one its status
    /// advertises, or if every workspace page is walked without finding an
    /// entry for that source path. Also returns any error
    /// [`AssetProcessorClient::workspace_entry_page`] raises while paging.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn refresh_current_build_status(mut self) -> EditorResult<Self> {
        let document_id = self
            .current_document_id
            .clone()
            .ok_or(EditorError::MissingGraphDocumentSelection)?;
        let source_path = document_id.as_str().trim().to_string();
        if source_path.is_empty() {
            return Err(EditorError::ServiceDiscovery(format!(
                "graph build status refresh found an empty source path for document `{}`",
                document_id.as_str()
            )));
        }
        let client = self.asset_processor_client().await?;
        let mut after_entry_id = None;
        loop {
            let result = client
                .workspace_entry_page(
                    AssetRootScope::All,
                    after_entry_id,
                    WORKSPACE_ENTRY_PAGE_SIZE,
                )
                .await?;
            if let Some(entry) = result
                .entries
                .iter()
                .find(|entry| entry.source_path == source_path)
            {
                self.build_status = Some(graph_build_status_from_asset_entry(&document_id, entry));
                return Ok(self);
            }
            let Some(next_after) = result.next_after_entry_id else {
                break;
            };
            after_entry_id = Some(next_after);
        }

        Err(EditorError::ServiceDiscovery(format!(
            "asset-processor attached workspace has no asset entry for graph source `{source_path}`"
        )))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn apply_planned_batch(
        mut self,
        batch: Option<GraphCommandBatchSnapshot>,
    ) -> EditorResult<Self> {
        let Some(batch) = batch else {
            return Ok(self);
        };
        let status = self
            .project_host_client("graph command apply")
            .await?
            .apply_graph_commands(&batch, &self.side_channel_root)
            .await?;
        let adapter = self
            .adapter
            .as_mut()
            .ok_or(EditorError::MissingGraphDocumentSelection)?;
        adapter.apply_project_host_status(&batch, &status)?;
        let document_id = adapter.document_id().clone();
        let revision = adapter.revision();
        let saved_revision = self.saved_revision;
        self.upsert_graph_document_entry(
            document_id,
            revision,
            saved_revision,
            saved_revision != Some(revision),
            true,
        );
        Ok(self)
    }

    fn first_graph_document_id(&self) -> Option<DocumentId> {
        self.graph_document_entries
            .iter()
            .find(|entry| self.is_graph_document_entry(entry))
            .map(|entry| entry.document_id.clone())
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn load_graph_document_entries(&self) -> EditorResult<Vec<GraphDocumentEntry>> {
        let client = self.asset_processor_client().await?;
        let mut documents_by_path = self
            .graph_document_entries
            .iter()
            .cloned()
            .map(|entry| (entry.source_path.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let mut after_entry_id = None;
        loop {
            let result = client
                .workspace_entry_page(
                    AssetRootScope::All,
                    after_entry_id,
                    WORKSPACE_ENTRY_PAGE_SIZE,
                )
                .await?;
            for asset in result.entries {
                let document_id = DocumentId::new(&asset.source_path);
                if !self.document_id_is_graph(&document_id) {
                    continue;
                }
                documents_by_path
                    .entry(asset.source_path.clone())
                    .or_insert(GraphDocumentEntry {
                        document_id,
                        source_path: asset.source_path,
                        revision: DocumentRevision::new(0),
                        saved_revision: Some(DocumentRevision::new(0)),
                        unsaved_changes: false,
                        loaded: false,
                    });
            }
            let Some(next_after) = result.next_after_entry_id else {
                break;
            };
            after_entry_id = Some(next_after);
        }
        let mut documents = documents_by_path.into_values().collect::<Vec<_>>();
        documents.sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));
        Ok(documents)
    }

    fn document_list_entry_for(&self, document_id: &DocumentId) -> Option<GraphDocumentEntry> {
        self.graph_document_entries
            .iter()
            .find(|entry| entry.document_id == *document_id)
            .cloned()
    }

    fn upsert_graph_document_entry(
        &mut self,
        document_id: DocumentId,
        revision: DocumentRevision,
        saved_revision: Option<DocumentRevision>,
        unsaved_changes: bool,
        loaded: bool,
    ) {
        let source_path = document_id.as_str().to_string();
        let entry = GraphDocumentEntry {
            document_id,
            source_path,
            revision,
            saved_revision,
            unsaved_changes,
            loaded,
        };
        if let Some(existing) = self
            .graph_document_entries
            .iter_mut()
            .find(|existing| existing.document_id == entry.document_id)
        {
            *existing = entry;
        } else {
            self.graph_document_entries.push(entry);
            self.graph_document_entries
                .sort_by(|left, right| left.document_id.as_str().cmp(right.document_id.as_str()));
        }
    }

    fn is_graph_document_entry(&self, entry: &GraphDocumentEntry) -> bool {
        self.document_id_is_graph(&entry.document_id)
    }

    fn document_id_is_graph(&self, document_id: &DocumentId) -> bool {
        let creation_catalog = graph_creation_catalog_from_graph_type_catalog(&self.graph_catalog);
        creation_catalog
            .graph_types
            .iter()
            .any(|graph_type| document_id_matches_graph_type(document_id, graph_type))
    }

    fn graph_type_for_document(&self, graph_type: &str) -> Option<&GraphTypeDescriptor> {
        self.graph_catalog.graph_type(&GraphTypeId::new(graph_type))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn asset_processor_client(&self) -> EditorResult<AssetProcessorClient> {
        let Some(session_slug) = &self.session_slug else {
            return Err(EditorError::ServiceDiscovery(
                "graph build status refresh requires an attached session slug".to_string(),
            ));
        };
        let Some(session_id) = self.session_id else {
            return Err(EditorError::ServiceDiscovery(
                "graph build status refresh requires an attached session id".to_string(),
            ));
        };
        let supervisor = self
            .session_supervisor_client("graph build status refresh")
            .await?;

        let descriptor = self
            .asset_processor_descriptor_from_supervisor(&supervisor, session_slug)
            .await?;
        AssetProcessorClient::connect_for_session(&descriptor, session_id).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn project_host_client(
        &self,
        operation: &'static str,
    ) -> EditorResult<ProjectHostClient> {
        #[cfg(test)]
        if self.supervisor.is_none()
            && self.supervisor_descriptor.is_none()
            && self.project_host_descriptor.is_none()
        {
            return Ok(self.client.clone());
        }

        if let Some(supervisor) = self
            .optional_session_supervisor_client(operation, true)
            .await?
        {
            let session_slug = self.attached_session_slug(operation)?;
            let status = supervisor.status(session_slug).await?;
            self.ensure_status_matches_attached_session(&status)?;
            let expected = status
                .manifest
                .services
                .iter()
                .find(|descriptor| {
                    descriptor.id.namespace == crate::project_host::PROJECT_HOST_SERVICE_NAMESPACE
                        && descriptor.id.name == crate::project_host::PROJECT_HOST_SERVICE_NAME
                        && descriptor.role == az_proto_core::ServiceRole::ProjectHost
                })
                .cloned()
                .ok_or_else(|| EditorError::MissingSessionService {
                    session: status.manifest.slug.clone(),
                    service: format!(
                        "{}/{}",
                        crate::project_host::PROJECT_HOST_SERVICE_NAMESPACE,
                        crate::project_host::PROJECT_HOST_SERVICE_NAME
                    ),
                })?;
            let resolved = supervisor
                .resolve_project_host_descriptor(session_slug)
                .await?;
            if !resolved.has_same_connection_contract(&expected) {
                return Err(graph_session_status_mismatch(
                    session_slug,
                    &format!(
                        "project-host descriptor from resolveService endpoint {:?} `{}` does not match status endpoint {:?} `{}` during {operation}",
                        resolved.endpoint.kind,
                        resolved.endpoint.address,
                        expected.endpoint.kind,
                        expected.endpoint.address
                    ),
                ));
            }
            let session_id = self.attached_session_id(operation)?;
            return ProjectHostClient::connect_for_session(&resolved, session_id).await;
        }

        let Some(descriptor) = &self.project_host_descriptor else {
            return Err(EditorError::ServiceDiscovery(format!(
                "{operation} requires a project-host descriptor"
            )));
        };
        let session_id = self.attached_session_id(operation)?;
        ProjectHostClient::connect_for_session(descriptor, session_id).await
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn optional_session_supervisor_client(
        &self,
        operation: &'static str,
        allow_missing: bool,
    ) -> EditorResult<Option<SessionSupervisorClient>> {
        #[cfg(test)]
        if let Some(supervisor) = &self.supervisor {
            return Ok(Some(supervisor.clone()));
        }

        let Some(descriptor) = &self.supervisor_descriptor else {
            if allow_missing {
                return Ok(None);
            }
            return Err(EditorError::ServiceDiscovery(format!(
                "{operation} requires a session-supervisor descriptor"
            )));
        };
        let session_id = self.attached_session_id(operation)?;
        Ok(Some(
            SessionSupervisorClient::connect_for_session(descriptor, session_id).await?,
        ))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn session_supervisor_client(
        &self,
        operation: &'static str,
    ) -> EditorResult<SessionSupervisorClient> {
        self.optional_session_supervisor_client(operation, false)
            .await?
            .ok_or_else(|| {
                EditorError::ServiceDiscovery(format!(
                    "{operation} requires a session-supervisor descriptor"
                ))
            })
    }

    fn attached_session_slug(&self, operation: &'static str) -> EditorResult<&str> {
        self.session_slug.as_deref().ok_or_else(|| {
            EditorError::ServiceDiscovery(format!("{operation} requires an attached session slug"))
        })
    }

    fn attached_session_id(&self, operation: &'static str) -> EditorResult<Uuid> {
        self.session_id.ok_or_else(|| {
            EditorError::ServiceDiscovery(format!("{operation} requires an attached session id"))
        })
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn asset_processor_descriptor_from_supervisor(
        &self,
        supervisor: &SessionSupervisorClient,
        session_slug: &str,
    ) -> EditorResult<ServiceDescriptor> {
        let status = supervisor.status(session_slug).await?;
        self.ensure_status_matches_attached_session(&status)?;
        let expected = asset_processor_descriptor_from_status(&status.manifest)?;
        let resolved = supervisor
            .resolve_asset_processor_descriptor(session_slug)
            .await?;
        if !resolved.has_same_connection_contract(&expected) {
            return Err(EditorError::ServiceDiscovery(format!(
                "asset-processor connection contract changed while resolving graph build status for session `{session_slug}`"
            )));
        }
        Ok(resolved)
    }

    fn ensure_status_matches_attached_session(
        &self,
        status: &ProtoSessionWorkspaceStatus,
    ) -> EditorResult<()> {
        let manifest = &status.manifest;
        if let Some(session_id) = self.session_id
            && manifest.id != session_id
        {
            return Err(graph_session_status_mismatch(
                &manifest.slug,
                &format!(
                    "status session id `{}` does not match attached session id `{}`",
                    manifest.id, session_id
                ),
            ));
        }
        if let Some(session_slug) = &self.session_slug
            && manifest.slug != *session_slug
        {
            return Err(graph_session_status_mismatch(
                &manifest.slug,
                &format!(
                    "status session slug `{}` does not match attached session slug `{}`",
                    manifest.slug, session_slug
                ),
            ));
        }
        Ok(())
    }
}

fn graph_node_from_descriptor_for_ui(
    descriptor: &NodeTypeDescriptor,
    layout: GraphNodeLayout,
) -> GraphNode {
    let mut node = GraphNode::new(
        GraphNodeId::new_v7(),
        descriptor.id.clone(),
        descriptor.version,
    );
    node.layout = layout;
    node.input_values = descriptor_default_input_values(descriptor);
    node
}

fn descriptor_default_input_values(
    descriptor: &NodeTypeDescriptor,
) -> BTreeMap<NodePortId, ReflectedValueEnvelope> {
    descriptor
        .ports
        .iter()
        .filter_map(|port| {
            if port.direction == NodePortDirection::Input
                && matches!(
                    port.value,
                    NodePortValue::Data { .. } | NodePortValue::DynamicData { .. }
                )
            {
                port.default_value
                    .as_ref()
                    .map(|value| (port.id, value.clone()))
            } else {
                None
            }
        })
        .collect()
}

fn graph_node_id_from_ui(node_id: &str) -> EditorResult<GraphNodeId> {
    let node_id = node_id.trim();
    let uuid =
        uuid::Uuid::parse_str(node_id).map_err(|source| EditorError::InvalidGraphNodeId {
            node_id: node_id.to_string(),
            reason: source.to_string(),
        })?;
    Ok(GraphNodeId::new(uuid))
}

fn graph_connection_id_from_ui(connection_id: &str) -> EditorResult<GraphConnectionId> {
    let connection_id = connection_id.trim();
    let uuid = uuid::Uuid::parse_str(connection_id).map_err(|source| {
        EditorError::InvalidGraphConnectionId {
            connection_id: connection_id.to_string(),
            reason: source.to_string(),
        }
    })?;
    Ok(GraphConnectionId::new(uuid))
}

fn graph_route_anchor_id_from_ui(anchor_id: &str) -> EditorResult<GraphRouteAnchorId> {
    let anchor_id = anchor_id.trim();
    let uuid = uuid::Uuid::parse_str(anchor_id).map_err(|source| {
        EditorError::InvalidGraphRouteAnchorId {
            anchor_id: anchor_id.to_string(),
            reason: source.to_string(),
        }
    })?;
    Ok(GraphRouteAnchorId::new(uuid))
}

fn graph_comment_id_from_ui(comment_id: &str) -> EditorResult<GraphCommentId> {
    let comment_id = comment_id.trim();
    let uuid =
        uuid::Uuid::parse_str(comment_id).map_err(|source| EditorError::InvalidGraphCommentId {
            comment_id: comment_id.to_string(),
            reason: source.to_string(),
        })?;
    Ok(GraphCommentId::new(uuid))
}

fn graph_port_id_from_ui(port_id: u32) -> EditorResult<NodePortId> {
    let port_id = NodePortId::new(port_id);
    if port_id.is_reserved() {
        return Err(EditorError::InvalidGraphPortId {
            port_id: port_id.0,
            reason: "port id 0 is reserved".to_string(),
        });
    }
    Ok(port_id)
}

fn graph_port_ref_from_ui(node_id: &str, port_id: u32) -> EditorResult<GraphPortRef> {
    Ok(GraphPortRef::new(
        graph_node_id_from_ui(node_id)?,
        graph_port_id_from_ui(port_id)?,
    ))
}

fn graph_session_status_mismatch(session: &str, reason: &str) -> EditorError {
    EditorError::ServiceDiscovery(format!(
        "session-supervisor status for session `{session}` does not match attached graph session: {reason}"
    ))
}

fn document_id_matches_graph_type(
    document_id: &DocumentId,
    graph_type: &EditorGraphTypeCreationData,
) -> bool {
    let path = document_id.as_str().replace('\\', "/");
    let extension = graph_type.default_extension.trim().trim_start_matches('.');
    if extension.is_empty() {
        return false;
    }
    let suffix = format!(".{extension}");
    if !path.ends_with(&suffix) {
        return false;
    }
    let prefix = graph_type
        .default_path_prefix
        .trim()
        .trim_matches('/')
        .trim_matches('\\');
    prefix.is_empty()
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/') && rest.len() > 1)
}

fn graph_creation_catalog_projection_from_graph_type_catalog(
    catalog: &GraphTypeCatalog,
) -> GraphCreationCatalogProjectionData {
    let graph_types = graph_creation_catalog_from_graph_type_catalog(catalog)
        .graph_types
        .into_iter()
        .map(graph_type_creation_projection_from_data)
        .collect();
    GraphCreationCatalogProjectionData::new(graph_types)
}

fn graph_type_creation_projection_from_descriptor(
    graph_type: &GraphTypeDescriptor,
) -> GraphTypeCreationProjectionData {
    graph_type_creation_projection_from_data(graph_type_creation_data(graph_type))
}

fn graph_type_creation_projection_from_data(
    graph_type: EditorGraphTypeCreationData,
) -> GraphTypeCreationProjectionData {
    GraphTypeCreationProjectionData {
        graph_type: graph_type.graph_type,
        label: graph_type.label,
        category: graph_type.category,
        default_path_prefix: graph_type.default_path_prefix,
        default_extension: graph_type.default_extension,
        compiler_backend: graph_type
            .compiler_backend
            .map(graph_compiler_backend_projection_data),
        runtime_product_asset_type: graph_type.runtime_product_asset_type,
        runtime_product_kind: graph_type.runtime_product_kind,
        runtime_product_streamable: graph_type.runtime_product_streamable,
        runtime_product_diffable_chunks: graph_type.runtime_product_diffable_chunks,
        runtime_execution_strategy: graph_type
            .runtime_execution_strategy
            .map(graph_runtime_execution_strategy_projection_data),
        runtime_compiled: graph_type.runtime_compiled,
        editor_interpreted: graph_type.editor_interpreted,
    }
}

fn graph_document_list_projection_from_entries(
    entries: &[GraphDocumentEntry],
    graph_catalog: &GraphTypeCatalog,
    current_document_id: Option<&DocumentId>,
) -> GraphDocumentListProjectionData {
    let creation_catalog = graph_creation_catalog_from_graph_type_catalog(graph_catalog);
    let mut documents = entries
        .iter()
        .filter_map(|entry| {
            let graph_type = creation_catalog.graph_types.iter().find(|graph_type| {
                document_id_matches_graph_type(&entry.document_id, graph_type)
            })?;
            Some(GraphDocumentListItemProjectionData {
                document_id: entry.document_id.as_str().to_string(),
                graph_type: graph_type.graph_type.clone(),
                source_path: entry.source_path.clone(),
                revision: entry.revision.0,
                saved_revision: entry.saved_revision.map(|revision| revision.0),
                unsaved_changes: entry.unsaved_changes,
                loaded: entry.loaded,
                current: current_document_id == Some(&entry.document_id),
            })
        })
        .collect::<Vec<_>>();
    documents.sort_by(|left, right| {
        left.document_id
            .cmp(&right.document_id)
            .then_with(|| left.graph_type.cmp(&right.graph_type))
    });
    GraphDocumentListProjectionData::new(documents)
}

fn graph_compiler_backend_projection_data(
    backend: EditorGraphCompilerBackendData,
) -> GraphCompilerBackendProjectionData {
    GraphCompilerBackendProjectionData {
        id: backend.id,
        kind: graph_compiler_backend_kind_projection_data(backend.kind),
        capability_markers: backend.capability_markers,
    }
}

fn graph_compiler_backend_kind_projection_data(
    kind: EditorGraphCompilerBackendKindData,
) -> GraphCompilerBackendKindProjectionData {
    match kind {
        EditorGraphCompilerBackendKindData::GeneratedRust {
            package,
            entry_symbol,
            abi,
        } => GraphCompilerBackendKindProjectionData::GeneratedRust {
            package,
            entry_symbol,
            abi: graph_generated_rust_abi_projection_data(abi),
        },
        EditorGraphCompilerBackendKindData::PackedIr { ir_schema } => {
            GraphCompilerBackendKindProjectionData::PackedIr { ir_schema }
        }
        EditorGraphCompilerBackendKindData::ShaderPipeline { pipeline_kind } => {
            GraphCompilerBackendKindProjectionData::ShaderPipeline { pipeline_kind }
        }
        EditorGraphCompilerBackendKindData::External { kind, locator } => {
            GraphCompilerBackendKindProjectionData::External { kind, locator }
        }
    }
}

const fn graph_generated_rust_abi_projection_data(
    abi: EditorGeneratedRustGraphAbiData,
) -> GraphGeneratedRustAbiProjectionData {
    match abi {
        EditorGeneratedRustGraphAbiData::ContextSchedule => {
            GraphGeneratedRustAbiProjectionData::ContextSchedule
        }
        EditorGeneratedRustGraphAbiData::TypedDataflow => {
            GraphGeneratedRustAbiProjectionData::TypedDataflow
        }
    }
}

fn graph_runtime_execution_strategy_projection_data(
    strategy: EditorGraphRuntimeExecutionStrategyData,
) -> GraphRuntimeExecutionStrategyProjectionData {
    match strategy {
        EditorGraphRuntimeExecutionStrategyData::PackedIr => {
            GraphRuntimeExecutionStrategyProjectionData::PackedIr
        }
        EditorGraphRuntimeExecutionStrategyData::AotCompiledCode {
            language,
            package,
            entry_symbol,
            context_type,
        } => GraphRuntimeExecutionStrategyProjectionData::AotCompiledCode {
            language,
            package,
            entry_symbol,
            context_type,
        },
        EditorGraphRuntimeExecutionStrategyData::HotReloadedCompiledModule {
            abi,
            entry_symbol,
        } => GraphRuntimeExecutionStrategyProjectionData::HotReloadedCompiledModule {
            abi,
            entry_symbol,
        },
        EditorGraphRuntimeExecutionStrategyData::ShaderPipeline { pipeline_kind } => {
            GraphRuntimeExecutionStrategyProjectionData::ShaderPipeline { pipeline_kind }
        }
        EditorGraphRuntimeExecutionStrategyData::External { kind, locator } => {
            GraphRuntimeExecutionStrategyProjectionData::External { kind, locator }
        }
    }
}

fn node_palette_projection_from_node_type_catalog(
    catalog: &NodeTypeCatalog,
    graph_type: Option<&GraphTypeDescriptor>,
) -> GraphNodePaletteProjectionData {
    let policy = graph_type.map(|graph_type| &graph_type.palette_policy);
    let mut nodes = catalog
        .node_types
        .iter()
        .filter(|node_type| {
            policy.is_none_or(|policy| node_type_matches_palette_policy(node_type, policy))
        })
        .map(node_palette_item_projection)
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.label.cmp(&right.label))
            .then_with(|| left.node_type.cmp(&right.node_type))
            .then_with(|| left.version.cmp(&right.version))
    });
    GraphNodePaletteProjectionData::new(nodes)
}

fn node_type_matches_palette_policy(
    node_type: &NodeTypeDescriptor,
    policy: &GraphPalettePolicy,
) -> bool {
    if node_type
        .tags
        .iter()
        .any(|tag| policy.hidden_node_tags.iter().any(|hidden| hidden == tag))
    {
        return false;
    }
    if !policy.root_categories.is_empty()
        && !policy
            .root_categories
            .iter()
            .any(|root| node_type_category_matches_root(&node_type.category_path, root))
    {
        return false;
    }
    policy
        .required_node_capabilities
        .iter()
        .all(|required| node_type_has_capability(node_type, required))
}

fn node_type_category_matches_root(category_path: &[String], root: &str) -> bool {
    let root = root.trim().trim_matches('/');
    if root.is_empty() {
        return true;
    }
    category_path
        .first()
        .is_some_and(|category| category == root)
        || category_path.join("/").starts_with(&format!("{root}/"))
        || category_path.join("/") == root
}

fn node_type_has_capability(node_type: &NodeTypeDescriptor, required: &str) -> bool {
    node_type.capabilities.iter().any(|capability| {
        capability.id == required || capability.markers.iter().any(|marker| marker == required)
    })
}

fn node_palette_item_projection(node_type: &NodeTypeDescriptor) -> GraphNodePaletteItemData {
    let input_count = node_type
        .ports
        .iter()
        .filter(|port| port.direction == NodePortDirection::Input)
        .count();
    let output_count = node_type.ports.len().saturating_sub(input_count);
    let default_input_count = node_type
        .ports
        .iter()
        .filter(|port| {
            port.direction == NodePortDirection::Input
                && port.value.is_data()
                && port.default_value.is_some()
        })
        .count();
    GraphNodePaletteItemData {
        node_type: node_type.id.as_str().to_string(),
        version: node_type.version,
        label: node_type.display_name.clone(),
        category: node_type.category_path.join("/"),
        description: node_type.description.clone(),
        input_count,
        output_count,
        default_input_count,
        runtime_bound: node_type.runtime_binding.is_some(),
        runtime_binding: node_type
            .runtime_binding
            .as_ref()
            .map(node_runtime_binding_projection),
        source_link_count: node_type.source_links.len(),
        tags: node_type.tags.clone(),
    }
}

fn node_runtime_binding_projection(
    binding: &NodeRuntimeBinding,
) -> GraphNodeRuntimeBindingProjectionData {
    match binding {
        NodeRuntimeBinding::RustSymbol {
            package,
            symbol,
            call_abi,
        } => GraphNodeRuntimeBindingProjectionData::RustSymbol {
            package: package.clone(),
            symbol: symbol.clone(),
            call_abi: rust_node_call_abi_projection(call_abi),
        },
        NodeRuntimeBinding::AssetBuilder { builder_id } => {
            GraphNodeRuntimeBindingProjectionData::AssetBuilder {
                builder_id: builder_id.clone(),
            }
        }
        NodeRuntimeBinding::RuntimeComponent { component_type } => {
            GraphNodeRuntimeBindingProjectionData::RuntimeComponent {
                component_type: component_type.clone(),
            }
        }
        NodeRuntimeBinding::External { kind, locator } => {
            GraphNodeRuntimeBindingProjectionData::External {
                kind: kind.clone(),
                locator: locator.clone(),
            }
        }
    }
}

fn rust_node_call_abi_projection(call_abi: &RustNodeCallAbi) -> GraphRustNodeCallAbiProjectionData {
    match call_abi {
        RustNodeCallAbi::ContextSchedule => GraphRustNodeCallAbiProjectionData::ContextSchedule,
        RustNodeCallAbi::TypedDataflow(dataflow) => {
            let input_parameter_count = dataflow
                .parameters
                .iter()
                .filter(|parameter| {
                    matches!(
                        parameter.source,
                        RustDataflowParameterSource::InputPort { .. }
                    )
                })
                .count();
            let by_value_parameter_count = dataflow
                .parameters
                .iter()
                .filter(|parameter| parameter.passing == RustValuePassing::ByValue)
                .count();
            let mutable_parameter_count = dataflow
                .parameters
                .iter()
                .filter(|parameter| parameter.passing == RustValuePassing::ByMutableRef)
                .count();
            GraphRustNodeCallAbiProjectionData::TypedDataflow {
                parameter_count: dataflow.parameters.len(),
                input_parameter_count,
                by_value_parameter_count,
                mutable_parameter_count,
                output_count: rust_dataflow_output_count(&dataflow.output),
                result: rust_call_result_projection(dataflow.result),
            }
        }
    }
}

const fn rust_dataflow_output_count(output: &RustDataflowOutput) -> usize {
    match output {
        RustDataflowOutput::None => 0,
        RustDataflowOutput::Single { .. } => 1,
        RustDataflowOutput::StructFields { fields, .. } => fields.len(),
    }
}

const fn rust_call_result_projection(result: RustCallResult) -> GraphRustCallResultProjectionData {
    match result {
        RustCallResult::Plain => GraphRustCallResultProjectionData::Plain,
        RustCallResult::Result => GraphRustCallResultProjectionData::Result,
    }
}

fn fresh_graph_batch_id(operation: &str) -> String {
    format!("az-editor-{operation}-{}", uuid::Uuid::now_v7())
}

/// Spawns the project-host round trip that resolves a node's source link and,
/// on success, opens it through the configured source-navigation settings.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] if no editor controller set
/// is installed, or [`EditorError::ControllerInstalling`],
/// [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the graph controller slot is not
/// ready. Everything after that is asynchronous: a failed resolve or a
/// source-navigation target that cannot be turned into a URL is logged and
/// published as a projection error, not returned here.
pub fn open_graph_node_source_link(
    cx: &mut App,
    action: az_editor_ui::actions::OpenGraphNodeSourceLink,
) -> EditorResult<()> {
    let attached = crate::controller_set::graph_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;
    let source_link = graph_node_source_link_from_action(action);
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "graph-source-link",
        move || async move {
            let client = controller
                .project_host_client("graph source-link resolve")
                .await?;
            client.resolve_node_source_link(source_link).await
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(target) => {
                    if let Err(err) = open_resolved_graph_node_source_link(cx, &target) {
                        let message = err.to_string();
                        error!(
                            error = %err,
                            "failed to open resolved graph source-link action"
                        );
                        Console::log_global(cx, LogLevel::Error, message);
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    error!(error = %err, "failed to resolve graph source link");
                    publish_graph_projection_error(cx, message);
                }
            }
        },
    );
    Ok(())
}

fn open_resolved_graph_node_source_link(
    cx: &mut App,
    target: &az_proto_project::NodeSourceLinkTarget,
) -> EditorResult<()> {
    let settings = cx
        .try_global::<SettingsStore>()
        .map_or_else(default_source_navigation_settings, |store| {
            store.current.source_navigation.clone()
        });
    let intent = source_navigation_intent(&settings, target)?;
    cx.open_url(&intent.url);
    info!(
        target = %intent.label,
        url = %intent.url,
        "opened graph node source-link action"
    );
    Console::log_global(
        cx,
        LogLevel::Info,
        format!("opened graph source: {}", intent.label),
    );
    cx.refresh_windows();
    Ok(())
}

fn graph_node_source_link_from_action(
    action: az_editor_ui::actions::OpenGraphNodeSourceLink,
) -> NodeSourceLink {
    NodeSourceLink {
        package: action.package,
        module_path: action.module_path,
        symbol_path: action.symbol_path,
        file: action.file,
        line: action.line,
        column: action.column,
        docs_url: action.docs_url,
    }
}

#[cfg(test)]
fn graph_node_source_link_label(link: &NodeSourceLink) -> String {
    let mut parts = Vec::new();
    if let Some(package) = link
        .package
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("package {package}"));
    }
    if let Some(symbol_path) = link
        .symbol_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("symbol {symbol_path}"));
    } else if let Some(module_path) = link
        .module_path
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("module {module_path}"));
    }
    if let Some(file) = link
        .file
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!(
            "file {}",
            graph_source_file_location(file, link.line, link.column)
        ));
    }
    if let Some(docs_url) = link
        .docs_url
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        parts.push(format!("docs {docs_url}"));
    }

    if parts.is_empty() {
        "descriptor source target".to_string()
    } else {
        parts.join(" | ")
    }
}

#[cfg(test)]
fn graph_source_file_location(file: &str, line: Option<u32>, column: Option<u32>) -> String {
    match (line, column) {
        (Some(line), Some(column)) => format!("{file}:{line}:{column}"),
        (Some(line), None) => format!("{file}:{line}"),
        _ => file.to_string(),
    }
}

pub fn install_graph_action_handlers(cx: &mut App) {
    install_graph_document_action_handlers(cx);
    install_graph_node_action_handlers(cx);
    install_graph_comment_action_handlers(cx);
}

/// Document-scoped actions: refresh, layout, save, build, create/open, and the
/// source-link jump. None of them mutate graph contents.
fn install_graph_document_action_handlers(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::RefreshGraphDocument, cx| {
        if let Err(err) = refresh_graph_document(cx) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph refresh action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::AutoLayoutGraph, cx| {
        if let Err(err) = auto_layout_current_graph(cx) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph auto-layout action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RouteGraphConnections, cx| {
        if let Err(err) = route_current_graph_connections(cx) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph route-connections action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::SaveGraphDocument, cx| {
        if let Err(err) = save_current_graph_document(cx) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph save action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::BuildGraphDocument, cx| {
        if let Err(err) = build_current_graph_document(cx) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph build action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|_: &az_editor_ui::actions::RefreshGraphBuildStatus, cx| {
        if let Err(err) = refresh_current_graph_build_status(cx) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph build-status refresh action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::CreateGraphDocument, cx| {
        if let Err(err) =
            create_graph_document(cx, action.graph_type.clone(), action.document_name.clone())
        {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph create action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::OpenGraphDocument, cx| {
        if let Err(err) = open_graph_document(cx, action.document_id.clone()) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph open action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(
        |action: &az_editor_ui::actions::OpenGraphNodeSourceLink, cx| {
            if let Err(err) = open_graph_node_source_link(cx, action.clone()) {
                let message = err.to_string();
                error!(error = %err, "failed to handle graph source-link action");
                publish_graph_projection_error(cx, message);
            }
        },
    );
}

/// Node-scoped edits: add, move, remove, reroute, connect, and set a literal
/// input value. Each one enqueues a graph controller action.
fn install_graph_node_action_handlers(cx: &mut App) {
    cx.on_action(|action: &az_editor_ui::actions::AddGraphNode, cx| {
        if let Err(err) = add_graph_node(
            cx,
            action.node_type.clone(),
            action.node_type_version,
            GraphNodeLayout {
                x: action.x,
                y: action.y,
            },
        ) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph add-node action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::MoveGraphNode, cx| {
        if let Err(err) = move_graph_node(
            cx,
            &action.node_id,
            GraphNodeLayout {
                x: action.x,
                y: action.y,
            },
        ) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph move-node action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::RemoveGraphNode, cx| {
        if let Err(err) = remove_graph_node(cx, &action.node_id) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph remove-node action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::MoveGraphRouteAnchor, cx| {
        if let Err(err) = move_graph_route_anchor(
            cx,
            &action.connection_id,
            &action.anchor_id,
            GraphPoint {
                x: action.x,
                y: action.y,
            },
        ) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph move-route-anchor action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::ConnectGraphPorts, cx| {
        if let Err(err) = connect_graph_ports(
            cx,
            &action.from_node_id,
            action.from_port_id,
            &action.to_node_id,
            action.to_port_id,
        ) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph connect-ports action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(
        |action: &az_editor_ui::actions::SetReflectedGraphPortValue, cx| {
            if let Err(err) =
                set_graph_input_value(cx, &action.node_id, action.port_id, action.value.clone())
            {
                let message = err.to_string();
                error!(error = %err, "failed to handle graph set-input-value action");
                publish_graph_projection_error(cx, message);
            }
        },
    );
}

/// Comment-scoped edits: create, move, retext, and remove.
fn install_graph_comment_action_handlers(cx: &mut App) {
    cx.on_action(|action: &az_editor_ui::actions::CreateGraphComment, cx| {
        if let Err(err) = create_graph_comment(
            cx,
            action.text.clone(),
            GraphCommentBounds {
                x: action.x,
                y: action.y,
                width: action.width,
                height: action.height,
            },
        ) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph create-comment action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::MoveGraphComment, cx| {
        if let Err(err) = move_graph_comment(
            cx,
            &action.comment_id,
            GraphCommentBounds {
                x: action.x,
                y: action.y,
                width: action.width,
                height: action.height,
            },
        ) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph move-comment action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::SetGraphCommentText, cx| {
        if let Err(err) = set_graph_comment_text(cx, &action.comment_id, action.text.clone()) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph set-comment-text action");
            publish_graph_projection_error(cx, message);
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::RemoveGraphComment, cx| {
        if let Err(err) = remove_graph_comment(cx, &action.comment_id) {
            let message = err.to_string();
            error!(error = %err, "failed to handle graph remove-comment action");
            publish_graph_projection_error(cx, message);
        }
    });
}

pub(crate) fn install_graph_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let session_slug = session.session_slug.clone();
    let load_session_slug = session_slug.clone();
    cx.set_global(EditorGraphDocumentProjection::empty());
    // Installing the queue identity here, before the connect RPC starts, is
    // what makes reattachment airtight: every path that reaches this installer
    // has just discarded the previous queue (a fresh aggregate on reattach, an
    // explicit retire on same-session retry), so a current graph fence always
    // names exactly one live queue and that queue always starts empty. The
    // install RPC below is not a queue item -- it produces the controller the
    // queue will start from.
    crate::controller_set::install_graph_action_queue(cx, fence);

    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "graph-install",
        move || {
            let load_session_label = load_session_slug.clone();
            async move {
                let controller = EditorGraphController::connect_attached(&session).await?;
                let controller = match controller.clone().refresh_current_or_first().await {
                    Ok(controller) => controller,
                    Err(err) => {
                        let message = err.to_string();
                        error!(
                            error = %err,
                            session = %load_session_label,
                            "failed to load initial graph document"
                        );
                        return Ok((controller, None, Some(message)));
                    }
                };
                let projection = controller.current_projection();
                Ok((controller, Some(projection), None))
            }
        },
        move |cx, result| match result {
            Ok((controller, projection, error_message)) => {
                let loaded = projection
                    .as_ref()
                    .is_some_and(|projection| projection.document.is_some());
                if !crate::controller_set::complete_graph(cx, fence, controller) {
                    return;
                }
                if let Some(projection) = projection {
                    cx.set_global(projection);
                }
                if let Some(message) = error_message {
                    publish_graph_projection_error(cx, message);
                }
                info!(
                    session = %session_slug,
                    graph_loaded = loaded,
                    "installed graph controller"
                );
            }
            Err(err) => {
                let message = err.to_string();
                crate::controller_set::fail_controller(cx, fence, message.clone());
                error!(
                    error = %err,
                    session = %session_slug,
                    "failed to connect graph controller"
                );
                publish_graph_projection_error(cx, message);
            }
        },
    );
}

/// The bounded number of graph controller actions one attached graph session
/// may hold pending.
///
/// The queue orders work; it is not a backlog absorber. Past this many
/// un-started actions the editor refuses new graph work with a typed error
/// rather than growing without limit, dropping work a user already asked for,
/// or coalescing two distinct edits into one.
pub(crate) const GRAPH_ACTION_QUEUE_CAPACITY: usize = 32;

/// One graph operation that replaces the attached [`EditorGraphController`] and
/// its published projection.
///
/// The enum owns every operation's typed inputs, task name, failure text,
/// execution, and optional console line, so a caller hands the queue a
/// described operation instead of an opaque controller closure. UI string
/// parsing happens before construction: an invalid id fails synchronously in
/// the action handler rather than occupying a queue slot and failing later.
pub(crate) enum GraphControllerAction {
    Refresh,
    AutoLayout,
    RouteConnections,
    Save,
    Build,
    RefreshBuildStatus,
    CreateDocument {
        graph_type: String,
        document_name: String,
    },
    OpenDocument {
        document_id: DocumentId,
    },
    AddNode {
        node_type: String,
        node_type_version: u32,
        layout: GraphNodeLayout,
    },
    MoveNode {
        node_id: GraphNodeId,
        layout: GraphNodeLayout,
    },
    RemoveNode {
        node_id: GraphNodeId,
    },
    MoveRouteAnchor {
        connection_id: GraphConnectionId,
        anchor_id: GraphRouteAnchorId,
        position: GraphPoint,
    },
    CreateComment {
        text: String,
        bounds: GraphCommentBounds,
    },
    MoveComment {
        comment_id: GraphCommentId,
        bounds: GraphCommentBounds,
    },
    SetCommentText {
        comment_id: GraphCommentId,
        text: String,
    },
    RemoveComment {
        comment_id: GraphCommentId,
    },
    ConnectPorts {
        from: GraphPortRef,
        to: GraphPortRef,
    },
    SetInputValue {
        node_id: GraphNodeId,
        port_id: NodePortId,
        value: Option<ReflectedValueEnvelope>,
    },
}

impl GraphControllerAction {
    /// The RPC task name this operation has always reported.
    const fn name(&self) -> &'static str {
        match self {
            Self::Refresh => "graph-refresh",
            Self::AutoLayout => "graph-auto-layout",
            Self::RouteConnections => "graph-route-connections",
            Self::Save => "graph-save",
            Self::Build => "graph-build",
            Self::RefreshBuildStatus => "graph-refresh-build-status",
            Self::CreateDocument { .. } => "graph-create-document",
            Self::OpenDocument { .. } => "graph-open-document",
            Self::AddNode { .. } => "graph-add-node",
            Self::MoveNode { .. } => "graph-move-node",
            Self::RemoveNode { .. } => "graph-remove-node",
            Self::MoveRouteAnchor { .. } => "graph-move-route-anchor",
            Self::CreateComment { .. } => "graph-create-comment",
            Self::MoveComment { .. } => "graph-move-comment",
            Self::SetCommentText { .. } => "graph-set-comment-text",
            Self::RemoveComment { .. } => "graph-remove-comment",
            Self::ConnectPorts { .. } => "graph-connect-ports",
            Self::SetInputValue { .. } => "graph-set-input-value",
        }
    }

    /// The log line this operation has always written when it fails.
    const fn failure_message(&self) -> &'static str {
        match self {
            Self::Refresh => "failed to refresh graph document",
            Self::AutoLayout => "failed to auto-layout graph document",
            Self::RouteConnections => "failed to route graph connections",
            Self::Save => "failed to save graph document",
            Self::Build => "failed to build graph document",
            Self::RefreshBuildStatus => "failed to refresh graph build status",
            Self::CreateDocument { .. } => "failed to create graph document",
            Self::OpenDocument { .. } => "failed to open graph document",
            Self::AddNode { .. } => "failed to add graph node",
            Self::MoveNode { .. } => "failed to move graph node",
            Self::RemoveNode { .. } => "failed to remove graph node",
            Self::MoveRouteAnchor { .. } => "failed to move graph route anchor",
            Self::CreateComment { .. } => "failed to create graph comment",
            Self::MoveComment { .. } => "failed to move graph comment",
            Self::SetCommentText { .. } => "failed to set graph comment text",
            Self::RemoveComment { .. } => "failed to remove graph comment",
            Self::ConnectPorts { .. } => "failed to connect graph ports",
            Self::SetInputValue { .. } => "failed to set graph input value",
        }
    }

    /// Runs the operation against the controller the driver took from the slot.
    ///
    /// This borrows rather than consumes the action so the same value can still
    /// describe its own success log once the operation has produced a
    /// controller. Every payload clone here is a small owned id or layout in
    /// front of a project-host round trip.
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn run(&self, controller: EditorGraphController) -> EditorResult<EditorGraphController> {
        match self {
            Self::Refresh => controller.refresh_current_or_first().await,
            Self::AutoLayout => controller.auto_layout_current().await,
            Self::RouteConnections => controller.route_current_connections().await,
            Self::Save => controller.save_current().await,
            Self::Build => controller.build_current().await,
            Self::RefreshBuildStatus => controller.refresh_current_build_status().await,
            Self::CreateDocument {
                graph_type,
                document_name,
            } => {
                controller
                    .create_graph_document(graph_type.clone(), document_name.clone())
                    .await
            }
            Self::OpenDocument { document_id } => {
                controller.load_graph_document(document_id.clone()).await
            }
            Self::AddNode {
                node_type,
                node_type_version,
                layout,
            } => {
                controller
                    .add_node_to_current(node_type.clone(), *node_type_version, *layout)
                    .await
            }
            Self::MoveNode { node_id, layout } => {
                controller.move_node_in_current(*node_id, *layout).await
            }
            Self::RemoveNode { node_id } => controller.remove_node_from_current(*node_id).await,
            Self::MoveRouteAnchor {
                connection_id,
                anchor_id,
                position,
            } => {
                controller
                    .move_route_anchor_in_current(*connection_id, *anchor_id, *position)
                    .await
            }
            Self::CreateComment { text, bounds } => {
                controller
                    .create_comment_in_current(text.clone(), *bounds)
                    .await
            }
            Self::MoveComment { comment_id, bounds } => {
                controller
                    .move_comment_in_current(*comment_id, *bounds)
                    .await
            }
            Self::SetCommentText { comment_id, text } => {
                controller
                    .set_comment_text_in_current(*comment_id, text.clone())
                    .await
            }
            Self::RemoveComment { comment_id } => {
                controller.remove_comment_from_current(*comment_id).await
            }
            Self::ConnectPorts { from, to } => {
                controller
                    .connect_ports_in_current(from.clone(), to.clone())
                    .await
            }
            Self::SetInputValue {
                node_id,
                port_id,
                value,
            } => {
                controller
                    .set_input_value_in_current(*node_id, *port_id, value.clone())
                    .await
            }
        }
    }

    /// The console line this operation writes after a successful publish.
    ///
    /// Only the two build-status operations report to the console, and both
    /// report the build status carried by the controller they produced.
    fn success_log(&self, controller: &EditorGraphController) -> Option<String> {
        match self {
            Self::Build | Self::RefreshBuildStatus => controller
                .build_status
                .as_ref()
                .map(graph_build_console_line),
            Self::Refresh
            | Self::AutoLayout
            | Self::RouteConnections
            | Self::Save
            | Self::CreateDocument { .. }
            | Self::OpenDocument { .. }
            | Self::AddNode { .. }
            | Self::MoveNode { .. }
            | Self::RemoveNode { .. }
            | Self::MoveRouteAnchor { .. }
            | Self::CreateComment { .. }
            | Self::MoveComment { .. }
            | Self::SetCommentText { .. }
            | Self::RemoveComment { .. }
            | Self::ConnectPorts { .. }
            | Self::SetInputValue { .. } => None,
        }
    }
}

/// Opaque identity of one attached graph session's action queue.
///
/// A completion carries the identity it started under. Nothing outside this
/// module can mint or forge one, so a late completion cannot talk its way into
/// a queue installed after it started.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GraphActionQueueIdentity(u64);

impl GraphActionQueueIdentity {
    /// Mints an identity no queue in this process has held before.
    ///
    /// The counter cannot live on the queue itself. Reattachment rebuilds the
    /// whole controller aggregate, so a per-aggregate counter would restart and
    /// the first queue of the new session would carry the same value as the
    /// first queue of the old one. A process-wide allocator is what makes
    /// "this completion belongs to the queue installed now" decidable from the
    /// identity alone, rather than only in combination with the controller
    /// fence.
    fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

struct ActiveGraphActionQueue {
    identity: GraphActionQueueIdentity,
    pending: VecDeque<GraphControllerAction>,
    in_flight: bool,
}

/// The outcome of offering one action to the live queue.
pub(crate) enum GraphActionAdmission {
    /// Accepted into an idle queue: the caller must start the driver for this
    /// identity.
    Start(GraphActionQueueIdentity),
    /// Accepted behind work a driver already owns; that driver reaches it in
    /// invocation order.
    Queued,
    /// The pending bound is reached. Work already accepted is untouched.
    Full,
    /// No attached graph session owns a queue for this fence.
    Retired,
}

/// Owns the serialized action queue of exactly one attached graph controller.
///
/// At most one graph RPC action runs at a time, and each one starts from the
/// controller the previous success published, so two overlapping mutations
/// plan their command batches against revisions R then R+1 instead of both
/// planning against R and having project-host reject one of them. Installing a
/// new controller retires the previous queue: its pending work is dropped and
/// its in-flight completion can no longer publish.
#[derive(Default)]
pub(crate) struct GraphActionQueue {
    active: Option<ActiveGraphActionQueue>,
}

impl GraphActionQueue {
    /// Retires any previous queue and installs a fresh, empty one.
    pub(crate) fn install(&mut self) -> GraphActionQueueIdentity {
        let identity = GraphActionQueueIdentity::next();
        self.active = Some(ActiveGraphActionQueue {
            identity,
            pending: VecDeque::new(),
            in_flight: false,
        });
        identity
    }

    pub(crate) fn is_current(&self, identity: GraphActionQueueIdentity) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.identity == identity)
    }

    pub(crate) fn push(&mut self, action: GraphControllerAction) -> GraphActionAdmission {
        let Some(active) = self.active.as_mut() else {
            return GraphActionAdmission::Retired;
        };
        if active.pending.len() >= GRAPH_ACTION_QUEUE_CAPACITY {
            return GraphActionAdmission::Full;
        }
        active.pending.push_back(action);
        if active.in_flight {
            GraphActionAdmission::Queued
        } else {
            GraphActionAdmission::Start(active.identity)
        }
    }

    pub(crate) fn start_next(
        &mut self,
        identity: GraphActionQueueIdentity,
    ) -> Option<GraphControllerAction> {
        let active = self
            .active
            .as_mut()
            .filter(|active| active.identity == identity)?;
        let action = active.pending.pop_front()?;
        active.in_flight = true;
        Some(action)
    }

    pub(crate) fn finish(&mut self, identity: GraphActionQueueIdentity) -> bool {
        let Some(active) = self
            .active
            .as_mut()
            .filter(|active| active.identity == identity)
        else {
            return false;
        };
        active.in_flight = false;
        true
    }

    /// Drops the queue with its pending work. A completion still in flight
    /// under the retired identity publishes nothing and advances nothing.
    pub(crate) fn retire(&mut self) {
        self.active = None;
    }

    #[cfg(test)]
    pub(crate) fn pending_len(&self) -> usize {
        self.active
            .as_ref()
            .map_or(0, |active| active.pending.len())
    }
}

/// The one way a graph operation reaches the attached controller.
///
/// Readiness is checked first so an unattached, installing, or failed graph
/// controller still fails synchronously at the call site exactly as before.
/// Accepted work is then ordered by the queue; the driver starts only when the
/// queue was idle, because an already running driver reaches the new action on
/// its own.
fn enqueue_graph_controller_action(
    cx: &mut App,
    action: GraphControllerAction,
) -> EditorResult<()> {
    let fence = crate::controller_set::ready_graph_controller_fence(cx)?;
    match crate::controller_set::enqueue_graph_action(cx, fence, action) {
        GraphActionAdmission::Start(identity) => start_next_graph_action(cx, fence, identity),
        GraphActionAdmission::Queued => {}
        GraphActionAdmission::Full => {
            return Err(EditorError::GraphActionQueueFull {
                capacity: GRAPH_ACTION_QUEUE_CAPACITY,
            });
        }
        GraphActionAdmission::Retired => {
            return Err(EditorError::MissingAttachedSession {
                operation: "queue a graph controller action",
            });
        }
    }
    Ok(())
}

/// Starts the next pending action against the controller installed right now.
///
/// Taking the controller here rather than at enqueue time is what makes the
/// queue a pipeline: action N+1 plans against whatever action N published.
fn start_next_graph_action(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    identity: GraphActionQueueIdentity,
) {
    let Some((action, controller)) =
        crate::controller_set::start_next_graph_action(cx, fence, identity)
    else {
        return;
    };
    let name = action.name();
    let failure_message = action.failure_message();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        name,
        move || async move {
            let controller = action.run(controller).await?;
            let console_line = action.success_log(&controller);
            Ok((controller, console_line))
        },
        move |cx, result| {
            complete_graph_action_in_app(cx, fence, identity, failure_message, result);
        },
    );
}

/// Publishes one completed action and hands the queue to the next one.
///
/// A success replaces the controller and its projection; a failure publishes
/// the typed error and leaves the last successful controller installed. Both
/// advance the queue, so one failed action never strands the work behind it.
fn complete_graph_action_in_app(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    identity: GraphActionQueueIdentity,
    failure_message: &'static str,
    result: EditorResult<(EditorGraphController, Option<String>)>,
) {
    if !crate::controller_set::graph_action_queue_is_current(cx, fence, identity) {
        // A reattach or a same-session retry retired this queue while the
        // action was in flight. The replacement identity owns its own driver,
        // so this completion neither publishes into it nor advances it.
        return;
    }
    match result {
        Ok((controller, console_line)) => {
            publish_graph_controller_update_in_app(cx, fence, controller);
            if let Some(console_line) = console_line {
                Console::log_global(cx, LogLevel::Info, console_line);
            }
        }
        Err(err) => publish_graph_action_error_in_app(cx, &err, failure_message),
    }
    if crate::controller_set::finish_graph_action(cx, fence, identity) {
        start_next_graph_action(cx, fence, identity);
    }
}

/// Queues a reload of the document list and the selected graph document.
///
/// # Errors
///
/// Returns [`EditorError::MissingAttachedSession`] if no editor controller set
/// is installed or the graph action queue was retired,
/// [`EditorError::ControllerInstalling`], [`EditorError::ControllerFailed`], or
/// [`EditorError::ControllerUnavailable`] when the graph controller slot is not
/// ready, and [`EditorError::GraphActionQueueFull`] when the queue is already
/// at capacity. The queued operation runs asynchronously; its own failure
/// reaches the console and the projection rather than this caller.
pub fn refresh_graph_document(cx: &mut App) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::Refresh)
}

/// Queues an automatic layout of the selected graph document.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn auto_layout_current_graph(cx: &mut App) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::AutoLayout)
}

/// Queues a re-route of every connection in the selected graph document.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn route_current_graph_connections(cx: &mut App) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::RouteConnections)
}

/// Queues a save of the selected graph document.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn save_current_graph_document(cx: &mut App) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::Save)
}

/// Queues a supervisor-side save of the selected graph document, which is what
/// enqueues its asset-builder job.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn build_current_graph_document(cx: &mut App) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::Build)
}

/// Queues a re-read of the asset-builder status for the selected document.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn refresh_current_graph_build_status(cx: &mut App) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::RefreshBuildStatus)
}

/// Queues creation of a new document of `graph_type` named `document_name`.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue. An
/// unpublished graph type and an unusable document name are rejected later by
/// [`EditorGraphController::create_graph_document`], not here.
pub fn create_graph_document(
    cx: &mut App,
    graph_type: String,
    document_name: String,
) -> EditorResult<()> {
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::CreateDocument {
            graph_type,
            document_name,
        },
    )
}

/// Queues a load of `document_id` as the graph panel's selection.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue. An unknown
/// document id is rejected later by
/// [`EditorGraphController::load_graph_document`], not here.
pub fn open_graph_document(cx: &mut App, document_id: String) -> EditorResult<()> {
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::OpenDocument {
            document_id: DocumentId::new(document_id),
        },
    )
}

/// Queues adding a node of `node_type` at `layout` to the selected document.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue. An
/// unpublished node type or version is rejected later by
/// [`EditorGraphController::add_node_to_current`], not here.
pub fn add_graph_node(
    cx: &mut App,
    node_type: String,
    node_type_version: u32,
    layout: GraphNodeLayout,
) -> EditorResult<()> {
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::AddNode {
            node_type,
            node_type_version,
            layout,
        },
    )
}

/// Queues a move of the node the UI names by `node_id` to `layout`.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphNodeId`] if `node_id` does not parse as a
/// UUID, then the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn move_graph_node(cx: &mut App, node_id: &str, layout: GraphNodeLayout) -> EditorResult<()> {
    let node_id = graph_node_id_from_ui(node_id)?;
    enqueue_graph_controller_action(cx, GraphControllerAction::MoveNode { node_id, layout })
}

/// Queues removal of the node the UI names by `node_id`.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphNodeId`] if `node_id` does not parse as a
/// UUID, then the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn remove_graph_node(cx: &mut App, node_id: &str) -> EditorResult<()> {
    let node_id = graph_node_id_from_ui(node_id)?;
    enqueue_graph_controller_action(cx, GraphControllerAction::RemoveNode { node_id })
}

/// Queues a drag of one connection route waypoint to `position`.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphConnectionId`] or
/// [`EditorError::InvalidGraphRouteAnchorId`] if the UI ids do not parse as
/// UUIDs, then the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn move_graph_route_anchor(
    cx: &mut App,
    connection_id: &str,
    anchor_id: &str,
    position: GraphPoint,
) -> EditorResult<()> {
    let connection_id = graph_connection_id_from_ui(connection_id)?;
    let anchor_id = graph_route_anchor_id_from_ui(anchor_id)?;
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::MoveRouteAnchor {
            connection_id,
            anchor_id,
            position,
        },
    )
}

/// Queues a new comment at `bounds` on the selected document.
///
/// # Errors
///
/// Returns the same admission errors as [`refresh_graph_document`]: an
/// unattached or unready graph controller, or a full action queue.
pub fn create_graph_comment(
    cx: &mut App,
    text: String,
    bounds: GraphCommentBounds,
) -> EditorResult<()> {
    enqueue_graph_controller_action(cx, GraphControllerAction::CreateComment { text, bounds })
}

/// Queues a move of the comment the UI names by `comment_id` to `bounds`.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphCommentId`] if `comment_id` does not
/// parse as a UUID, then the same admission errors as
/// [`refresh_graph_document`]: an unattached or unready graph controller, or a
/// full action queue.
pub fn move_graph_comment(
    cx: &mut App,
    comment_id: &str,
    bounds: GraphCommentBounds,
) -> EditorResult<()> {
    let comment_id = graph_comment_id_from_ui(comment_id)?;
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::MoveComment { comment_id, bounds },
    )
}

/// Queues a text replacement on the comment the UI names by `comment_id`.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphCommentId`] if `comment_id` does not
/// parse as a UUID, then the same admission errors as
/// [`refresh_graph_document`]: an unattached or unready graph controller, or a
/// full action queue.
pub fn set_graph_comment_text(cx: &mut App, comment_id: &str, text: String) -> EditorResult<()> {
    let comment_id = graph_comment_id_from_ui(comment_id)?;
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::SetCommentText { comment_id, text },
    )
}

/// Queues removal of the comment the UI names by `comment_id`.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphCommentId`] if `comment_id` does not
/// parse as a UUID, then the same admission errors as
/// [`refresh_graph_document`]: an unattached or unready graph controller, or a
/// full action queue.
pub fn remove_graph_comment(cx: &mut App, comment_id: &str) -> EditorResult<()> {
    let comment_id = graph_comment_id_from_ui(comment_id)?;
    enqueue_graph_controller_action(cx, GraphControllerAction::RemoveComment { comment_id })
}

/// Queues a connection between two ports the UI names by node id and port id.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphNodeId`] if either node id does not parse
/// as a UUID, [`EditorError::InvalidGraphPortId`] if either port id is the
/// reserved id 0, then the same admission errors as [`refresh_graph_document`]:
/// an unattached or unready graph controller, or a full action queue. Whether
/// the two ports may actually be connected is decided later by
/// [`EditorGraphController::connect_ports_in_current`].
pub fn connect_graph_ports(
    cx: &mut App,
    from_node_id: &str,
    from_port_id: u32,
    to_node_id: &str,
    to_port_id: u32,
) -> EditorResult<()> {
    let from = graph_port_ref_from_ui(from_node_id, from_port_id)?;
    let to = graph_port_ref_from_ui(to_node_id, to_port_id)?;
    enqueue_graph_controller_action(cx, GraphControllerAction::ConnectPorts { from, to })
}

/// Queues a literal input-value assignment, or a clear with `None`, on one
/// port the UI names by node id and port id.
///
/// # Errors
///
/// Returns [`EditorError::InvalidGraphNodeId`] if `node_id` does not parse as a
/// UUID, [`EditorError::InvalidGraphPortId`] if `port_id` is the reserved id 0,
/// then the same admission errors as [`refresh_graph_document`]: an unattached
/// or unready graph controller, or a full action queue.
pub fn set_graph_input_value(
    cx: &mut App,
    node_id: &str,
    port_id: u32,
    value: Option<ReflectedValueEnvelope>,
) -> EditorResult<()> {
    let node_id = graph_node_id_from_ui(node_id)?;
    let port_id = graph_port_id_from_ui(port_id)?;
    enqueue_graph_controller_action(
        cx,
        GraphControllerAction::SetInputValue {
            node_id,
            port_id,
            value,
        },
    )
}

fn publish_graph_controller_update_in_app(
    cx: &mut App,
    fence: crate::controller_set::ControllerFence,
    controller: EditorGraphController,
) {
    let projection = controller.current_projection();
    if !crate::controller_set::replace_graph(cx, fence, controller) {
        return;
    }
    cx.set_global(projection);
}

fn publish_graph_action_error_in_app(cx: &mut App, err: &EditorError, message: &'static str) {
    let error_message = err.to_string();
    error!(error = %err, "{message}");
    publish_graph_projection_error(cx, error_message);
}

pub fn publish_graph_projection_error(cx: &mut App, error: impl Into<String>) {
    let error = error.into();
    let projection = match cx.try_global::<EditorGraphDocumentProjection>().cloned() {
        Some(mut projection) => {
            projection.status_error = Some(error);
            projection
        }
        None => EditorGraphDocumentProjection::error(error),
    };
    cx.set_global(projection);
}

#[cfg(test)]
mod tests {
    use az_graph_layout::{
        DefaultGraphLayoutSolver, GraphAutoLayoutOptions, GraphLayoutDirection, GraphLayoutScope,
    };
    use az_node_graph::{
        GraphCompilerBackendDescriptor, GraphConnection, GraphNode, GraphNodeCatalogRequirement,
        GraphPalettePolicy, GraphPortRef, GraphSourceWorkflow, GraphTypeCatalog,
        GraphTypeDescriptor, NodeCapability, NodePortCapacity, NodePortDescriptor,
        NodePortDirection, NodePortId, NodePortValue, NodeRuntimeBinding, NodeTypeCatalog,
        NodeTypeDescriptor, RuntimeGraphProductDescriptor, RustDataflowParameter,
        RustTypedDataflowNodeCall,
    };
    use az_proto_project::GraphDiagnosticSeverity;
    use uuid::Uuid;

    use super::*;

    const FLOAT_SCHEMA: &str = "core.f32";

    #[test]
    fn graph_creation_catalog_projects_registered_graph_type_workflows() {
        let catalog = GraphTypeCatalog::new(
            1,
            123,
            vec![
                graph_type_descriptor("az.editor.tests.logic-b", "Logic B", "graphs-b"),
                graph_type_descriptor("az.editor.tests.logic-a", "Logic A", "graphs-a"),
            ],
        );

        let creation = graph_creation_catalog_from_graph_type_catalog(&catalog);

        assert_eq!(creation.graph_types.len(), 2);
        assert_eq!(
            creation.graph_types[0].graph_type,
            "az.editor.tests.logic-a"
        );
        assert_eq!(creation.graph_types[0].label, "Logic A");
        assert_eq!(creation.graph_types[0].category, "Tests/Logic");
        assert_eq!(creation.graph_types[0].default_path_prefix, "graphs-a");
        assert_eq!(creation.graph_types[0].default_extension, "azgraph.ron");
        assert_eq!(
            creation.graph_types[0].source_workflow_kind,
            EditorGraphSourceWorkflowKindData::File
        );
        assert_eq!(
            creation.graph_types[0].compiler_backend,
            Some(EditorGraphCompilerBackendData {
                id: "az.editor.tests.logic-a.compiler".to_string(),
                kind: EditorGraphCompilerBackendKindData::PackedIr {
                    ir_schema: "azoth.graph.logic-ir/v1".to_string(),
                },
                capability_markers: vec!["zero-cost".to_string()],
            })
        );
        assert_eq!(
            creation.graph_types[0].runtime_product_kind.as_deref(),
            Some("azoth.graph.logic-ir")
        );
        assert_eq!(
            creation.graph_types[0]
                .runtime_product_asset_type
                .as_deref(),
            Some("azoth.graph.packed-ir")
        );
        assert_eq!(
            creation.graph_types[0].runtime_product_streamable,
            Some(true)
        );
        assert_eq!(
            creation.graph_types[0].runtime_product_diffable_chunks,
            Some(true)
        );
        assert_eq!(
            creation.graph_types[0].runtime_execution_strategy,
            Some(EditorGraphRuntimeExecutionStrategyData::PackedIr)
        );
        assert!(creation.graph_types[0].runtime_compiled);
        assert!(!creation.graph_types[0].editor_interpreted);
    }

    #[test]
    fn graph_creation_catalog_projects_to_gpui_creation_options() {
        let catalog = GraphTypeCatalog::new(
            1,
            123,
            vec![graph_type_descriptor(
                "az.editor.tests.logic",
                "Logic",
                "graphs",
            )],
        );

        let projection = graph_creation_catalog_projection_from_graph_type_catalog(&catalog);

        assert_eq!(projection.graph_types.len(), 1);
        assert_eq!(
            projection.graph_types[0].graph_type,
            "az.editor.tests.logic"
        );
        assert_eq!(projection.graph_types[0].label, "Logic");
        assert_eq!(projection.graph_types[0].category, "Tests/Logic");
        assert_eq!(projection.graph_types[0].default_path_prefix, "graphs");
        assert_eq!(projection.graph_types[0].default_extension, "azgraph.ron");
        assert_eq!(
            projection.graph_types[0].compiler_backend,
            Some(GraphCompilerBackendProjectionData {
                id: "az.editor.tests.logic.compiler".to_string(),
                kind: GraphCompilerBackendKindProjectionData::PackedIr {
                    ir_schema: "azoth.graph.logic-ir/v1".to_string(),
                },
                capability_markers: vec!["zero-cost".to_string()],
            })
        );
        assert_eq!(
            projection.graph_types[0].runtime_product_kind.as_deref(),
            Some("azoth.graph.logic-ir")
        );
        assert_eq!(
            projection.graph_types[0]
                .runtime_product_asset_type
                .as_deref(),
            Some("azoth.graph.packed-ir")
        );
        assert_eq!(
            projection.graph_types[0].runtime_product_streamable,
            Some(true)
        );
        assert_eq!(
            projection.graph_types[0].runtime_product_diffable_chunks,
            Some(true)
        );
        assert_eq!(
            projection.graph_types[0].runtime_execution_strategy,
            Some(GraphRuntimeExecutionStrategyProjectionData::PackedIr)
        );
        assert!(projection.graph_types[0].runtime_compiled);
        assert!(!projection.graph_types[0].editor_interpreted);
    }

    #[test]
    fn graph_document_list_projection_filters_to_graph_documents_and_marks_current() {
        let graph_catalog = GraphTypeCatalog::new(
            1,
            123,
            vec![graph_type_descriptor(
                "az.editor.tests.logic",
                "Logic",
                "graphs",
            )],
        );
        let current = DocumentId::new("graphs/combat.azgraph.ron");
        let entries = vec![
            graph_document_entry("graphs/combat.azgraph.ron", 7, Some(6), true),
            graph_document_entry("prefabs/tree.prefab.ron", 1, Some(1), false),
        ];

        let projection =
            graph_document_list_projection_from_entries(&entries, &graph_catalog, Some(&current));

        assert_eq!(projection.documents.len(), 1);
        assert_eq!(
            projection.documents[0].document_id,
            "graphs/combat.azgraph.ron"
        );
        assert_eq!(projection.documents[0].graph_type, "az.editor.tests.logic");
        assert_eq!(projection.documents[0].revision, 7);
        assert_eq!(projection.documents[0].saved_revision, Some(6));
        assert!(projection.documents[0].unsaved_changes);
        assert!(projection.documents[0].current);
    }

    #[test]
    fn graph_build_status_projects_session_save_asset_record() {
        let save = SaveGraphDocumentResult {
            saved: az_proto_project::SavedDocument {
                document_id: DocumentId::new("graphs/combat.azgraph.ron"),
                revision: DocumentRevision::new(4),
                source_path: "graphs/combat.azgraph.ron".to_string(),
                schema_type: "azoth.graph.visual".to_string(),
                content_hash: vec![0xab; 32],
                byte_length: 1024,
            },
            asset_record: az_proto_asset::SourceAssetRecordResult {
                asset_guid: uuid::uuid!("8d8d3389-8f6a-42dc-82b2-2b35f7ff1726"),
                entry: az_proto_asset::WorkspaceEntry {
                    entry_id: 7,
                    workspace_id: 3,
                    asset_guid: uuid::uuid!("8d8d3389-8f6a-42dc-82b2-2b35f7ff1726"),
                    root_id: 5,
                    source_path: "graphs/combat.azgraph.ron".to_string(),
                    schema_type: Some("azoth.graph.visual".to_string()),
                    content_hash: "ab".repeat(32),
                    diff: az_proto_asset::WorkspaceEntryDiff::Added,
                    diagnostics_count: 0,
                    updated_unix_ms: 123,
                    jobs: vec![az_proto_asset::JobActivity {
                        job: az_proto_asset::JobRecord {
                            job_id: 41,
                            workspace_id: 3,
                            source_guid: uuid::uuid!("8d8d3389-8f6a-42dc-82b2-2b35f7ff1726"),
                            source_path: "graphs/combat.azgraph.ron".to_string(),
                            source_root: std::env::temp_dir()
                                .join("azoth-graph-ui-project")
                                .to_string_lossy()
                                .into_owned(),
                            source_schema_type: Some("azoth.graph.visual".to_string()),
                            owner: az_proto_asset::JobOwner::Build(uuid::uuid!(
                                "e622e48c-5c8a-4b28-a64e-e080d5aa51f1"
                            )),
                            key: "compile-graph-runtime-product".to_string(),
                            platform: "pc".to_string(),
                            status: az_proto_asset::JobStatus::Leased,
                            ready: true,
                            attempts: 1,
                        },
                        attempt: Some(az_proto_asset::JobAttemptRecord {
                            attempt_id: 42,
                            job_id: 41,
                            ordinal: 1,
                            status: az_proto_asset::AttemptStatus::Leased,
                            owner: None,
                            staging: None,
                            finished_unix_ms: None,
                            error_count: 0,
                            warning_count: 0,
                        }),
                    }],
                },
            },
        };

        let status = graph_build_status_from_save_result(&save);

        assert_eq!(status.document_id, "graphs/combat.azgraph.ron");
        assert_eq!(status.source_path, "graphs/combat.azgraph.ron");
        assert_eq!(status.asset_guid, "8d8d3389-8f6a-42dc-82b2-2b35f7ff1726");
        assert_eq!(status.source_status, GraphBuildSourceStatusData::Added);
        assert_eq!(status.entry_id, 7);
        assert_eq!(
            status.latest_job,
            Some(GraphBuildJobProjectionData {
                job_id: 41,
                attempt_id: Some(42),
                job_key: "compile-graph-runtime-product".to_string(),
                platform: "pc".to_string(),
                ordinal: Some(1),
                status: GraphBuildJobStatusData::Leased,
                error_count: 0,
                warning_count: 0,
            })
        );
        assert_eq!(
            graph_build_console_line(&status),
            "graph build status: graphs/combat.azgraph.ron -> compile-graph-runtime-product:pc #1 leased"
        );
    }

    #[test]
    fn node_palette_projection_applies_current_graph_palette_policy() {
        let visible = NodeTypeDescriptor::new("az.editor.tests.visible", 1, "Visible")
            .with_category_path(["Logic".to_string(), "Flow".to_string()])
            .with_port(
                data_input_descriptor(1, FLOAT_SCHEMA)
                    .with_default_value(reflected_value(FLOAT_SCHEMA, "1.0")),
            )
            .with_port(data_output_descriptor(2, FLOAT_SCHEMA))
            .with_capability(NodeCapability::new("azoth.node.logic").with_marker("debug"))
            .with_runtime_binding(NodeRuntimeBinding::rust_symbol(
                "az_editor_tests",
                "visible",
            ));
        let hidden = NodeTypeDescriptor::new("az.editor.tests.hidden", 1, "Hidden")
            .with_category_path(["Logic".to_string()])
            .with_capability(NodeCapability::new("azoth.node.logic"))
            .with_tag("internal");
        let wrong_category = NodeTypeDescriptor::new("az.editor.tests.material", 1, "Material")
            .with_category_path(["Material".to_string()])
            .with_capability(NodeCapability::new("azoth.node.logic"));
        let missing_capability = NodeTypeDescriptor::new("az.editor.tests.utility", 1, "Utility")
            .with_category_path(["Logic".to_string()]);
        let catalog = NodeTypeCatalog::new(
            1,
            123,
            vec![hidden, wrong_category, visible, missing_capability],
        );
        let graph_type = graph_type_descriptor("az.editor.tests.logic", "Logic", "graphs")
            .with_palette_policy(
                GraphPalettePolicy::default()
                    .with_root_category("Logic")
                    .with_required_node_capability("azoth.node.logic")
                    .with_hidden_node_tag("internal"),
            );

        let palette = node_palette_projection_from_node_type_catalog(&catalog, Some(&graph_type));

        assert_eq!(palette.nodes.len(), 1);
        assert_eq!(palette.nodes[0].node_type, "az.editor.tests.visible");
        assert_eq!(palette.nodes[0].category, "Logic/Flow");
        assert_eq!(palette.nodes[0].input_count, 1);
        assert_eq!(palette.nodes[0].output_count, 1);
        assert_eq!(palette.nodes[0].default_input_count, 1);
        assert!(palette.nodes[0].runtime_bound);
        assert_eq!(
            palette.nodes[0].runtime_binding,
            Some(GraphNodeRuntimeBindingProjectionData::RustSymbol {
                package: "az_editor_tests".to_string(),
                symbol: "visible".to_string(),
                call_abi: GraphRustNodeCallAbiProjectionData::ContextSchedule,
            })
        );
    }

    #[test]
    fn node_palette_projection_preserves_rust_typed_dataflow_call_abi() {
        let node = NodeTypeDescriptor::new("az.editor.tests.add", 1, "Add")
            .with_category_path(["Logic".to_string(), "Math".to_string()])
            .with_port(data_input_descriptor(1, FLOAT_SCHEMA))
            .with_port(data_input_descriptor(2, FLOAT_SCHEMA))
            .with_port(data_output_descriptor(3, FLOAT_SCHEMA))
            .with_runtime_binding(NodeRuntimeBinding::rust_typed_dataflow(
                "az_editor_tests",
                "az_editor_tests::math::add",
                RustTypedDataflowNodeCall::new(RustDataflowOutput::Single {
                    port: NodePortId::new(3),
                    rust_type: "f32".to_string(),
                })
                .with_parameter(RustDataflowParameter::runtime_context(
                    "&mut az_editor_tests::LogicContext",
                    RustValuePassing::ByMutableRef,
                ))
                .with_parameter(RustDataflowParameter::input(
                    NodePortId::new(1),
                    "f32",
                    RustValuePassing::ByValue,
                ))
                .with_parameter(RustDataflowParameter::input(
                    NodePortId::new(2),
                    "f32",
                    RustValuePassing::BySharedRef,
                ))
                .with_result(RustCallResult::Result),
            ));
        let catalog = NodeTypeCatalog::new(1, 123, vec![node]);

        let palette = node_palette_projection_from_node_type_catalog(&catalog, None);

        assert_eq!(palette.nodes.len(), 1);
        assert_eq!(
            palette.nodes[0].runtime_binding,
            Some(GraphNodeRuntimeBindingProjectionData::RustSymbol {
                package: "az_editor_tests".to_string(),
                symbol: "az_editor_tests::math::add".to_string(),
                call_abi: GraphRustNodeCallAbiProjectionData::TypedDataflow {
                    parameter_count: 3,
                    input_parameter_count: 2,
                    by_value_parameter_count: 1,
                    mutable_parameter_count: 1,
                    output_count: 1,
                    result: GraphRustCallResultProjectionData::Result,
                },
            })
        );
    }

    #[test]
    fn graph_node_from_descriptor_initializes_descriptor_defaults() {
        let descriptor = NodeTypeDescriptor::new("az.editor.tests.defaulted", 1, "Defaulted")
            .with_port(
                data_input_descriptor(1, FLOAT_SCHEMA)
                    .with_default_value(reflected_value(FLOAT_SCHEMA, "42.0")),
            )
            .with_port(data_output_descriptor(2, FLOAT_SCHEMA));

        let node =
            graph_node_from_descriptor_for_ui(&descriptor, GraphNodeLayout { x: 12.0, y: 34.0 });

        assert_eq!(node.node_type.as_str(), "az.editor.tests.defaulted");
        assert_eq!(node.node_type_version, 1);
        assert_eq!(node.layout, GraphNodeLayout { x: 12.0, y: 34.0 });
        assert_eq!(node.input_values.len(), 1);
        assert_eq!(
            node.input_values.get(&NodePortId::new(1)),
            Some(&reflected_value(FLOAT_SCHEMA, "42.0"))
        );
        assert!(!node.input_values.contains_key(&NodePortId::new(2)));
    }

    #[test]
    fn graph_node_id_from_ui_rejects_invalid_ids_before_project_host_rpc() {
        assert!(matches!(
            graph_node_id_from_ui("not-a-uuid"),
            Err(EditorError::InvalidGraphNodeId { .. })
        ));

        let node_id = graph_node_id_from_ui("018f0000-0000-7000-8000-000000000001").unwrap();
        assert_eq!(node_id.to_string(), "018f0000-0000-7000-8000-000000000001");
    }

    #[test]
    fn graph_port_ref_from_ui_parses_node_and_port_identity() {
        let port_ref = graph_port_ref_from_ui("018f0000-0000-7000-8000-000000000001", 7).unwrap();

        assert_eq!(
            port_ref.node_id.to_string(),
            "018f0000-0000-7000-8000-000000000001"
        );
        assert_eq!(port_ref.port_id, NodePortId::new(7));
    }

    #[test]
    fn graph_connection_and_route_anchor_ids_from_ui_reject_invalid_ids() {
        assert!(matches!(
            graph_connection_id_from_ui("not-a-uuid"),
            Err(EditorError::InvalidGraphConnectionId { .. })
        ));
        assert!(matches!(
            graph_route_anchor_id_from_ui("not-a-uuid"),
            Err(EditorError::InvalidGraphRouteAnchorId { .. })
        ));
        assert!(matches!(
            graph_comment_id_from_ui("not-a-uuid"),
            Err(EditorError::InvalidGraphCommentId { .. })
        ));

        assert_eq!(
            graph_connection_id_from_ui("018f0000-0000-7000-8000-000000000003")
                .unwrap()
                .to_string(),
            "018f0000-0000-7000-8000-000000000003"
        );
        assert_eq!(
            graph_route_anchor_id_from_ui("018f0000-0000-7000-8000-000000000004")
                .unwrap()
                .to_string(),
            "018f0000-0000-7000-8000-000000000004"
        );
        assert_eq!(
            graph_comment_id_from_ui("018f0000-0000-7000-8000-000000000005")
                .unwrap()
                .to_string(),
            "018f0000-0000-7000-8000-000000000005"
        );
    }

    #[test]
    fn graph_port_id_from_ui_rejects_reserved_zero() {
        assert!(matches!(
            graph_port_id_from_ui(0),
            Err(EditorError::InvalidGraphPortId { .. })
        ));
    }

    #[test]
    fn graph_creation_catalog_preserves_generated_rust_backend_abi_and_runtime_entry() {
        let catalog = GraphTypeCatalog::new(
            1,
            123,
            vec![generated_rust_graph_type_descriptor(
                "az.editor.tests.combat-rust",
                "Combat Rust",
                "graphs",
            )],
        );

        let creation = graph_creation_catalog_from_graph_type_catalog(&catalog);
        let graph_type = creation
            .graph_type("az.editor.tests.combat-rust")
            .expect("generated Rust graph type projected");

        assert_eq!(
            graph_type.compiler_backend,
            Some(EditorGraphCompilerBackendData {
                id: "az.editor.tests.combat-rust.compiler".to_string(),
                kind: EditorGraphCompilerBackendKindData::GeneratedRust {
                    package: "az_editor_tests".to_string(),
                    entry_symbol: "run_combat_graph".to_string(),
                    abi: EditorGeneratedRustGraphAbiData::TypedDataflow,
                },
                capability_markers: vec!["zero-cost".to_string()],
            })
        );
        assert_eq!(
            graph_type.runtime_execution_strategy,
            Some(EditorGraphRuntimeExecutionStrategyData::AotCompiledCode {
                language: "rust".to_string(),
                package: "az_editor_tests".to_string(),
                entry_symbol: "run_combat_graph".to_string(),
                context_type: "az_editor_tests::CombatGraphContext".to_string(),
            })
        );
    }

    #[test]
    fn graph_creation_catalog_preserves_shader_pipeline_backend() {
        let catalog = GraphTypeCatalog::new(
            1,
            123,
            vec![shader_graph_type_descriptor(
                "az.editor.tests.material",
                "Material",
                "materials",
            )],
        );

        let creation = graph_creation_catalog_from_graph_type_catalog(&catalog);
        let graph_type = creation
            .graph_type("az.editor.tests.material")
            .expect("shader graph type projected");

        assert_eq!(
            graph_type.compiler_backend,
            Some(EditorGraphCompilerBackendData {
                id: "az.editor.tests.material.compiler".to_string(),
                kind: EditorGraphCompilerBackendKindData::ShaderPipeline {
                    pipeline_kind: "azoth.render.material".to_string(),
                },
                capability_markers: Vec::new(),
            })
        );
        assert_eq!(
            graph_type.runtime_execution_strategy,
            Some(EditorGraphRuntimeExecutionStrategyData::ShaderPipeline {
                pipeline_kind: "azoth.render.material".to_string(),
            })
        );
    }

    #[test]
    fn graph_creation_catalog_derives_default_project_document_ids() {
        let catalog = graph_creation_catalog_from_graph_type_catalog(&GraphTypeCatalog::new(
            1,
            123,
            vec![graph_type_descriptor(
                "az.editor.tests.logic",
                "Logic",
                "graphs",
            )],
        ));
        let entry = catalog.graph_type("az.editor.tests.logic").unwrap();

        let document_id = graph_document_id_from_creation_data(entry, "combat").unwrap();

        assert_eq!(document_id.as_str(), "graphs/combat.azgraph.ron");
    }

    #[test]
    fn graph_creation_catalog_rejects_path_like_document_names() {
        let catalog = graph_creation_catalog_from_graph_type_catalog(&GraphTypeCatalog::new(
            1,
            123,
            vec![graph_type_descriptor(
                "az.editor.tests.logic",
                "Logic",
                "graphs",
            )],
        ));
        let entry = catalog.graph_type("az.editor.tests.logic").unwrap();

        assert!(matches!(
            graph_document_id_from_creation_data(entry, "../combat"),
            Err(EditorGraphCreationError::InvalidDocumentName { .. })
        ));
        assert!(matches!(
            graph_document_id_from_creation_data(entry, ""),
            Err(EditorGraphCreationError::EmptyDocumentName)
        ));
    }

    #[test]
    fn graph_ui_adapter_plans_graph_commands_without_mutating_snapshot() {
        let mut adapter = test_adapter();
        let node = node_id(1);

        let batch = adapter
            .plan_move_node("move-node", node, GraphNodeLayout { x: 320.0, y: 16.0 })
            .unwrap();

        assert_eq!(batch.document_id.as_str(), "graphs/test.azgraph.ron");
        assert_eq!(batch.expected_revision, Some(DocumentRevision::new(2)));
        assert_eq!(batch.commands.len(), 1);
        assert_eq!(
            adapter
                .snapshot()
                .document
                .nodes
                .iter()
                .find(|candidate| candidate.id == node)
                .unwrap()
                .layout,
            GraphNodeLayout { x: 0.0, y: 0.0 }
        );

        let accepted = GraphCommandStatusSnapshot {
            document_id: batch.document_id.clone(),
            client_batch_id: batch.client_batch_id.clone(),
            applied_command_count: u32::try_from(batch.commands.len())
                .expect("command count fits u32"),
            outcome: GraphCommandStatusOutcome::Accepted {
                revision: DocumentRevision::new(3),
            },
            diagnostics: Vec::new(),
        };

        assert!(
            adapter
                .apply_project_host_status(&batch, &accepted)
                .unwrap()
        );
        assert_eq!(adapter.revision(), DocumentRevision::new(3));
        assert_eq!(
            adapter
                .snapshot()
                .document
                .nodes
                .iter()
                .find(|candidate| candidate.id == node)
                .unwrap()
                .layout,
            GraphNodeLayout { x: 320.0, y: 16.0 }
        );
    }

    #[test]
    fn graph_ui_adapter_plans_connect_command_without_constructing_in_ui() {
        let mut snapshot = test_snapshot();
        snapshot.document.connections.clear();
        let adapter =
            EditorGraphUiAdapter::new(snapshot, test_catalog(), GraphGeometrySnapshot::default())
                .unwrap();

        let batch = adapter
            .plan_connect_ports(
                "connect",
                GraphPortRef::new(node_id(1), NodePortId::new(2)),
                GraphPortRef::new(node_id(2), NodePortId::new(1)),
            )
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        let GraphCommand::Connect { connection } = &batch.commands[0] else {
            panic!("expected connect command");
        };
        assert_eq!(connection.from.node_id, node_id(1));
        assert_eq!(connection.from.port_id, NodePortId::new(2));
        assert_eq!(connection.to.node_id, node_id(2));
        assert_eq!(connection.to.port_id, NodePortId::new(1));
    }

    #[test]
    fn graph_ui_adapter_plans_set_input_value_command() {
        let adapter = test_adapter();

        let batch = adapter
            .plan_set_input_value(
                "set-input",
                node_id(1),
                NodePortId::new(1),
                Some(reflected_value(FLOAT_SCHEMA, "7.5")),
            )
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        let GraphCommand::SetInputValue {
            node_id: command_node_id,
            port_id,
            value,
        } = &batch.commands[0]
        else {
            panic!("expected set-input-value command");
        };
        assert_eq!(*command_node_id, node_id(1));
        assert_eq!(*port_id, NodePortId::new(1));
        assert_eq!(value, &Some(reflected_value(FLOAT_SCHEMA, "7.5")));
    }

    #[test]
    fn graph_ui_adapter_plans_remove_node_and_incident_connections() {
        let adapter = test_adapter();
        let expected_node_id = node_id(1);

        let batch = adapter
            .plan_remove_node("remove-node", expected_node_id)
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        assert!(matches!(
            batch.commands[0],
            GraphCommand::RemoveNode { node_id } if node_id == expected_node_id
        ));

        let mut projected = adapter.snapshot().document.clone();
        projected
            .apply_commands(batch.commands, adapter.catalog())
            .unwrap();
        assert_eq!(projected.nodes.len(), 1);
        assert!(projected.connections.is_empty());
    }

    #[test]
    fn graph_ui_adapter_plans_route_anchor_move_without_losing_route_metadata() {
        let mut snapshot = test_snapshot();
        snapshot.document.connections[0].route = GraphConnectionRoute::orthogonal().with_anchor(
            az_node_graph::GraphRouteAnchor::user_waypoint(
                route_anchor_id(10),
                GraphPoint::new(40.0, 50.0),
            )
            .with_outgoing_segment(az_node_graph::GraphRouteSegmentConstraint::Fixed),
        );
        let adapter =
            EditorGraphUiAdapter::new(snapshot, test_catalog(), GraphGeometrySnapshot::default())
                .unwrap();

        let batch = adapter
            .plan_move_route_anchor(
                "move-route-anchor",
                connection_id(1),
                route_anchor_id(10),
                GraphPoint::new(72.0, 96.0),
            )
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        let GraphCommand::SetConnectionRoute {
            connection_id: command_connection_id,
            route,
        } = &batch.commands[0]
        else {
            panic!("expected set-connection-route command");
        };
        assert_eq!(*command_connection_id, connection_id(1));
        assert_eq!(route.style, az_node_graph::GraphRouteStyle::Orthogonal);
        assert_eq!(route.anchors.len(), 1);
        assert_eq!(route.anchors[0].id, route_anchor_id(10));
        assert_eq!(route.anchors[0].position, GraphPoint::new(72.0, 96.0));
        assert_eq!(
            route.anchors[0].outgoing_segment,
            az_node_graph::GraphRouteSegmentConstraint::Fixed
        );
    }

    #[test]
    fn graph_ui_adapter_rejects_unknown_route_anchor_move() {
        let adapter = test_adapter();

        assert!(matches!(
            adapter.plan_move_route_anchor(
                "move-route-anchor",
                connection_id(1),
                route_anchor_id(404),
                GraphPoint::new(72.0, 96.0),
            ),
            Err(EditorGraphUiAdapterError::UnknownRouteAnchor { .. })
        ));
    }

    #[test]
    fn graph_ui_adapter_plans_comment_create_command() {
        let adapter = test_adapter();
        let comment = GraphComment {
            id: comment_id(10),
            text: "Combat note".to_string(),
            bounds: GraphCommentBounds {
                x: 12.0,
                y: 24.0,
                width: 220.0,
                height: 96.0,
            },
        };

        let batch = adapter
            .plan_create_comment("create-comment", comment.clone())
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        let GraphCommand::UpsertComment {
            comment: command_comment,
        } = &batch.commands[0]
        else {
            panic!("expected upsert-comment command");
        };
        assert_eq!(command_comment, &comment);
    }

    #[test]
    fn graph_ui_adapter_plans_comment_move_without_losing_text() {
        let mut snapshot = test_snapshot();
        snapshot.document.comments.push(GraphComment {
            id: comment_id(10),
            text: "Keep this text".to_string(),
            bounds: GraphCommentBounds {
                x: 12.0,
                y: 24.0,
                width: 220.0,
                height: 96.0,
            },
        });
        let adapter =
            EditorGraphUiAdapter::new(snapshot, test_catalog(), GraphGeometrySnapshot::default())
                .unwrap();

        let batch = adapter
            .plan_move_comment(
                "move-comment",
                comment_id(10),
                GraphCommentBounds {
                    x: 40.0,
                    y: 64.0,
                    width: 240.0,
                    height: 112.0,
                },
            )
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        let GraphCommand::UpsertComment { comment } = &batch.commands[0] else {
            panic!("expected upsert-comment command");
        };
        assert_eq!(comment.id, comment_id(10));
        assert_eq!(comment.text, "Keep this text");
        assert_eq!(
            comment.bounds,
            GraphCommentBounds {
                x: 40.0,
                y: 64.0,
                width: 240.0,
                height: 112.0,
            }
        );
    }

    #[test]
    fn graph_ui_adapter_plans_comment_text_update_without_losing_bounds() {
        let mut snapshot = test_snapshot();
        snapshot.document.comments.push(GraphComment {
            id: comment_id(10),
            text: "Before".to_string(),
            bounds: GraphCommentBounds {
                x: 12.0,
                y: 24.0,
                width: 220.0,
                height: 96.0,
            },
        });
        let adapter =
            EditorGraphUiAdapter::new(snapshot, test_catalog(), GraphGeometrySnapshot::default())
                .unwrap();

        let batch = adapter
            .plan_set_comment_text("set-comment-text", comment_id(10), "After".to_string())
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        let GraphCommand::UpsertComment { comment } = &batch.commands[0] else {
            panic!("expected upsert-comment command");
        };
        assert_eq!(comment.id, comment_id(10));
        assert_eq!(comment.text, "After");
        assert_eq!(
            comment.bounds,
            GraphCommentBounds {
                x: 12.0,
                y: 24.0,
                width: 220.0,
                height: 96.0,
            }
        );
    }

    #[test]
    fn graph_ui_adapter_rejects_unknown_comment_move() {
        let adapter = test_adapter();

        assert!(matches!(
            adapter.plan_move_comment(
                "move-comment",
                comment_id(404),
                GraphCommentBounds {
                    x: 40.0,
                    y: 64.0,
                    width: 240.0,
                    height: 112.0,
                },
            ),
            Err(EditorGraphUiAdapterError::UnknownComment { .. })
        ));
        assert!(matches!(
            adapter.plan_set_comment_text(
                "set-comment-text",
                comment_id(404),
                "missing".to_string(),
            ),
            Err(EditorGraphUiAdapterError::UnknownComment { .. })
        ));
    }

    #[test]
    fn graph_ui_adapter_plans_comment_remove_command() {
        let mut snapshot = test_snapshot();
        let expected_comment_id = comment_id(10);
        snapshot.document.comments.push(GraphComment {
            id: expected_comment_id,
            text: "Remove me".to_string(),
            bounds: GraphCommentBounds {
                x: 12.0,
                y: 24.0,
                width: 220.0,
                height: 96.0,
            },
        });
        let adapter =
            EditorGraphUiAdapter::new(snapshot, test_catalog(), GraphGeometrySnapshot::default())
                .unwrap();

        let batch = adapter
            .plan_remove_comment("remove-comment", expected_comment_id)
            .unwrap();

        assert_eq!(batch.commands.len(), 1);
        assert!(matches!(
            batch.commands[0],
            GraphCommand::RemoveComment { comment_id } if comment_id == expected_comment_id
        ));
    }

    #[test]
    fn graph_ui_adapter_does_not_apply_rejected_batches() {
        let mut adapter = test_adapter();
        let batch = adapter
            .plan_move_node(
                "move-rejected",
                node_id(1),
                GraphNodeLayout { x: 80.0, y: 64.0 },
            )
            .unwrap();
        let rejected = GraphCommandStatusSnapshot {
            document_id: batch.document_id.clone(),
            client_batch_id: batch.client_batch_id.clone(),
            applied_command_count: 0,
            outcome: GraphCommandStatusOutcome::Rejected {
                command_index: Some(0),
                reason: "validation failed".to_string(),
            },
            diagnostics: vec![az_proto_project::GraphCommandDiagnostic {
                command_index: Some(0),
                severity: GraphDiagnosticSeverity::Error,
                message: "validation failed".to_string(),
            }],
        };

        assert!(
            !adapter
                .apply_project_host_status(&batch, &rejected)
                .unwrap()
        );
        assert_eq!(adapter.revision(), DocumentRevision::new(2));
        assert_eq!(
            adapter
                .snapshot()
                .document
                .nodes
                .iter()
                .find(|candidate| candidate.id == node_id(1))
                .unwrap()
                .layout,
            GraphNodeLayout { x: 0.0, y: 0.0 }
        );
    }

    #[test]
    fn graph_ui_adapter_rejects_mismatched_status() {
        let mut adapter = test_adapter();
        let batch = adapter
            .plan_move_node("move-node", node_id(1), GraphNodeLayout { x: 32.0, y: 0.0 })
            .unwrap();
        let status = GraphCommandStatusSnapshot {
            document_id: DocumentId::new("graphs/other.azgraph.ron"),
            client_batch_id: batch.client_batch_id.clone(),
            applied_command_count: u32::try_from(batch.commands.len())
                .expect("command count fits u32"),
            outcome: GraphCommandStatusOutcome::Accepted {
                revision: DocumentRevision::new(3),
            },
            diagnostics: Vec::new(),
        };

        assert!(matches!(
            adapter.apply_project_host_status(&batch, &status),
            Err(EditorGraphUiAdapterError::StatusMismatch(_))
        ));
    }

    #[test]
    fn graph_ui_adapter_plans_layout_batches_from_engine_solver() {
        let adapter = test_adapter();
        let solver = DefaultGraphLayoutSolver::default();
        let batch = adapter
            .plan_layout(
                &solver,
                GraphLayoutOperation::AutoLayout(GraphAutoLayoutOptions {
                    direction: GraphLayoutDirection::LeftToRight,
                    scope: GraphLayoutScope::WholeDocument,
                }),
                "auto-layout",
            )
            .unwrap()
            .expect("layout should move and route the graph");

        assert_eq!(batch.document_id, *adapter.document_id());
        assert_eq!(batch.expected_revision, Some(adapter.revision()));
        assert!(!batch.commands.is_empty());
        assert!(
            batch
                .commands
                .iter()
                .any(|command| matches!(command, GraphCommand::MoveNode { .. }))
        );
    }

    #[test]
    fn graph_ui_adapter_spatial_index_tracks_measured_node_geometry() {
        let mut adapter = test_adapter();
        let expected_node = node_id(1);
        adapter.set_node_bounds(expected_node, GraphRect::new(20.0, 30.0, 220.0, 96.0));

        let hits = adapter.query_rect(GraphRect::new(24.0, 34.0, 4.0, 4.0));
        assert!(hits
            .iter()
            .any(|entry| matches!(entry.kind, az_graph_layout::GraphSpatialEntryKind::Node { node_id } if node_id == expected_node)));
    }

    #[test]
    // Every geometry assertion below compares against the exact literal the
    // fixture assigned (or an exact halving of it), so equality is the point.
    #[allow(clippy::float_cmp)] // projected geometry is compared to the exact fixture constants it came from.
    fn graph_ui_adapter_projects_validated_snapshot_for_gpui_panel() {
        let mut adapter = test_adapter();
        adapter.set_node_bounds(node_id(1), GraphRect::new(0.0, 0.0, 260.0, 120.0));
        adapter.snapshot.document.nodes[0]
            .input_values
            .insert(NodePortId::new(1), reflected_value(FLOAT_SCHEMA, "4.0"));
        adapter.snapshot.document.comments.push(GraphComment {
            id: comment_id(10),
            text: "Review combat branch".to_string(),
            bounds: GraphCommentBounds {
                x: 20.0,
                y: 40.0,
                width: 240.0,
                height: 96.0,
            },
        });

        let graph_type = graph_type_descriptor("azoth.graph.test", "Test Graph", "graphs");
        let projection =
            adapter.to_ui_projection(Some(DocumentRevision::new(2)), Some(&graph_type));
        let document = projection.document.expect("graph document projection");

        assert_eq!(document.document_id, "graphs/test.azgraph.ron");
        assert_eq!(document.graph_type, "azoth.graph.test");
        let graph_type_info = document
            .graph_type_info
            .as_ref()
            .expect("selected document graph type projection");
        assert_eq!(graph_type_info.label, "Test Graph");
        assert_eq!(graph_type_info.default_path_prefix, "graphs");
        assert_eq!(
            graph_type_info
                .compiler_backend
                .as_ref()
                .expect("compiler backend")
                .capability_markers,
            vec!["zero-cost".to_string()]
        );
        assert_eq!(
            graph_type_info.runtime_execution_strategy,
            Some(GraphRuntimeExecutionStrategyProjectionData::PackedIr)
        );
        assert_eq!(document.revision, 2);
        assert_eq!(document.saved_revision, Some(2));
        assert!(!document.unsaved_changes);
        assert_eq!(document.catalog_version, 1);
        assert_eq!(document.nodes.len(), 2);
        assert_eq!(document.nodes[0].label, "Float");
        assert_eq!(document.nodes[0].width, 260.0);
        assert_eq!(document.nodes[0].height, 120.0);
        assert_eq!(document.nodes[0].source_links.len(), 1);
        assert_eq!(
            document.nodes[0].source_links[0].symbol_path.as_deref(),
            Some("az_editor_tests::nodes::FloatNode::run")
        );
        assert_eq!(
            document.nodes[0].source_links[0].file.as_deref(),
            Some("crates/editor/tests/src/nodes.rs")
        );
        assert_eq!(document.nodes[0].ports.len(), 2);
        assert_eq!(document.nodes[0].ports[0].name, "in");
        assert_eq!(
            document.nodes[0].ports[0].direction,
            GraphPortDirectionData::Input
        );
        assert_eq!(document.nodes[0].ports[0].side, GraphPortSideData::West);
        assert_eq!(document.nodes[0].ports[0].x, 0.0);
        assert_eq!(document.nodes[0].ports[0].y, 60.0);
        assert_eq!(
            document.nodes[0].ports[0].value,
            Some(GraphInputValueProjectionData {
                schema_type: FLOAT_SCHEMA.to_string(),
                current_value: Some(reflected_value(FLOAT_SCHEMA, "4.0")),
                default_value: None,
            })
        );
        assert_eq!(document.nodes[0].ports[1].name, "out");
        assert_eq!(
            document.nodes[0].ports[1].direction,
            GraphPortDirectionData::Output
        );
        assert_eq!(document.nodes[0].ports[1].side, GraphPortSideData::East);
        assert_eq!(document.nodes[0].ports[1].x, 260.0);
        assert_eq!(document.nodes[0].ports[1].y, 60.0);
        assert_eq!(document.connections.len(), 1);
        assert_eq!(document.connections[0].points.len(), 2);
        assert_eq!(document.connections[0].route_anchors.len(), 2);
        assert_eq!(
            document.connections[0].route_anchors[0].kind,
            GraphRouteAnchorKindData::PortEndpoint
        );
        assert_eq!(document.comments.len(), 1);
        assert_eq!(document.comments[0].comment_id, comment_id(10).to_string());
        assert_eq!(document.comments[0].text, "Review combat branch");
        assert_eq!(document.comments[0].x, 20.0);
        assert_eq!(document.comments[0].width, 240.0);
    }

    #[test]
    fn graph_source_link_label_keeps_descriptor_target_visible() {
        let link =
            graph_node_source_link_from_action(az_editor_ui::actions::OpenGraphNodeSourceLink {
                package: Some("az-editor-tests".to_string()),
                module_path: Some("az_editor_tests::nodes".to_string()),
                symbol_path: Some("az_editor_tests::nodes::FloatNode::run".to_string()),
                file: Some("crates/editor/tests/src/nodes.rs".to_string()),
                line: Some(12),
                column: Some(5),
                docs_url: None,
            });

        assert_eq!(
            graph_node_source_link_label(&link),
            "package az-editor-tests | symbol az_editor_tests::nodes::FloatNode::run | file crates/editor/tests/src/nodes.rs:12:5"
        );
    }

    fn test_adapter() -> EditorGraphUiAdapter {
        EditorGraphUiAdapter::new(
            test_snapshot(),
            test_catalog(),
            GraphGeometrySnapshot::default(),
        )
        .unwrap()
    }

    fn graph_type_descriptor(
        id: &'static str,
        label: &'static str,
        path_prefix: &'static str,
    ) -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            id,
            1,
            label,
            GraphSourceWorkflow::file(format!("{id}.source"), "azgraph.ron")
                .with_default_path_prefix(path_prefix),
            GraphCompilerBackendDescriptor::packed_ir(
                format!("{id}.compiler"),
                "azoth.graph.logic-ir/v1",
            )
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.packed-ir",
                "azoth.graph.logic-ir",
                RuntimeGraphExecutionStrategy::PackedIr,
            ),
        )
        .with_category_path(["Tests".to_string(), "Logic".to_string()])
        .with_node_catalog(GraphNodeCatalogRequirement::new("az.editor.tests.nodes"))
        .with_palette_policy(GraphPalettePolicy::default().with_root_category("Logic"))
        .with_tag("test")
    }

    fn generated_rust_graph_type_descriptor(
        id: &'static str,
        label: &'static str,
        path_prefix: &'static str,
    ) -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            id,
            1,
            label,
            GraphSourceWorkflow::file(format!("{id}.source"), "azgraph.ron")
                .with_default_path_prefix(path_prefix),
            GraphCompilerBackendDescriptor::generated_rust_typed_dataflow(
                format!("{id}.compiler"),
                "az_editor_tests",
                "run_combat_graph",
            )
            .with_capability_marker("zero-cost"),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.aot-manifest",
                "azoth.graph.generated-rust",
                RuntimeGraphExecutionStrategy::aot_compiled_rust(
                    "az_editor_tests",
                    "run_combat_graph",
                    "az_editor_tests::CombatGraphContext",
                ),
            ),
        )
        .with_category_path(["Tests".to_string(), "Logic".to_string()])
        .with_node_catalog(GraphNodeCatalogRequirement::new("az.editor.tests.nodes"))
        .with_palette_policy(GraphPalettePolicy::default().with_root_category("Logic"))
        .with_tag("test")
    }

    fn shader_graph_type_descriptor(
        id: &'static str,
        label: &'static str,
        path_prefix: &'static str,
    ) -> GraphTypeDescriptor {
        GraphTypeDescriptor::runtime_compiled(
            id,
            1,
            label,
            GraphSourceWorkflow::file(format!("{id}.source"), "azmat.ron")
                .with_default_path_prefix(path_prefix),
            GraphCompilerBackendDescriptor::shader_pipeline(
                format!("{id}.compiler"),
                "azoth.render.material",
            ),
            RuntimeGraphProductDescriptor::new(
                "azoth.graph.shader-pipeline",
                "azoth.render.material",
                RuntimeGraphExecutionStrategy::shader_pipeline("azoth.render.material"),
            ),
        )
        .with_category_path(["Tests".to_string(), "Rendering".to_string()])
        .with_node_catalog(GraphNodeCatalogRequirement::new("az.editor.tests.nodes"))
        .with_palette_policy(GraphPalettePolicy::default().with_root_category("Material"))
        .with_tag("test")
    }

    fn test_snapshot() -> GraphDocumentSnapshot {
        let mut source = GraphNode::new(node_id(1), "azoth.test.float", 1);
        source.layout = GraphNodeLayout { x: 0.0, y: 0.0 };
        let mut target = GraphNode::new(node_id(2), "azoth.test.float", 1);
        target.layout = GraphNodeLayout { x: 500.0, y: 0.0 };
        GraphDocumentSnapshot {
            document_id: DocumentId::new("graphs/test.azgraph.ron"),
            revision: DocumentRevision::new(2),
            document: az_node_graph::VisualGraphDocument {
                document_version: 1,
                graph_type: "azoth.graph.test".to_string(),
                required_catalog_hash: None,
                nodes: vec![source.clone(), target.clone()],
                connections: vec![GraphConnection::new(
                    connection_id(1),
                    GraphPortRef::new(source.id, NodePortId::new(2)),
                    GraphPortRef::new(target.id, NodePortId::new(1)),
                )],
                comments: Vec::new(),
            },
        }
    }

    fn test_catalog() -> NodeTypeCatalog {
        NodeTypeCatalog::new(
            1,
            100,
            vec![
                NodeTypeDescriptor::new("azoth.test.float", 1, "Float")
                    .with_port(NodePortDescriptor::new(
                        NodePortId::new(1),
                        "in",
                        NodePortDirection::Input,
                        NodePortValue::Data {
                            schema_type: FLOAT_SCHEMA.to_string(),
                        },
                    ))
                    .with_port(
                        NodePortDescriptor::new(
                            NodePortId::new(2),
                            "out",
                            NodePortDirection::Output,
                            NodePortValue::Data {
                                schema_type: FLOAT_SCHEMA.to_string(),
                            },
                        )
                        .with_capacity(NodePortCapacity::Multiple),
                    )
                    .with_source_link(NodeSourceLink::rust_symbol(
                        "az-editor-tests",
                        "az_editor_tests::nodes",
                        "az_editor_tests::nodes::FloatNode::run",
                        "crates/editor/tests/src/nodes.rs",
                        12,
                        5,
                    )),
            ],
        )
    }

    fn data_input_descriptor(id: u32, schema_type: &str) -> NodePortDescriptor {
        NodePortDescriptor::new(
            NodePortId::new(id),
            "in",
            NodePortDirection::Input,
            NodePortValue::Data {
                schema_type: schema_type.to_string(),
            },
        )
    }

    fn reflected_value(type_path: &str, payload: &str) -> ReflectedValueEnvelope {
        ReflectedValueEnvelope::typed_ron(type_path, payload)
    }

    fn data_output_descriptor(id: u32, schema_type: &str) -> NodePortDescriptor {
        NodePortDescriptor::new(
            NodePortId::new(id),
            "out",
            NodePortDirection::Output,
            NodePortValue::Data {
                schema_type: schema_type.to_string(),
            },
        )
    }

    fn node_id(value: u128) -> GraphNodeId {
        GraphNodeId::new(Uuid::from_u128(value))
    }

    fn connection_id(value: u128) -> GraphConnectionId {
        GraphConnectionId::new(Uuid::from_u128(value))
    }

    fn route_anchor_id(value: u128) -> GraphRouteAnchorId {
        GraphRouteAnchorId::new(Uuid::from_u128(value))
    }

    fn comment_id(value: u128) -> GraphCommentId {
        GraphCommentId::new(Uuid::from_u128(value))
    }

    fn graph_document_entry(
        document_id: &'static str,
        revision: u64,
        saved_revision: Option<u64>,
        unsaved_changes: bool,
    ) -> GraphDocumentEntry {
        GraphDocumentEntry {
            document_id: DocumentId::new(document_id),
            source_path: document_id.to_string(),
            revision: DocumentRevision::new(revision),
            saved_revision: saved_revision.map(DocumentRevision::new),
            unsaved_changes,
            loaded: true,
        }
    }

    const QUEUE_NODE_TYPE: &str = "az.editor.tests.Print";
    const QUEUE_DOCUMENT_ID: &str = "graphs/queue.visual.ron";

    /// An `EditorGraphController` wired to the in-process project-host, holding
    /// a freshly created graph document at revision 0.
    ///
    /// The returned client is a second handle on the same host: tests read the
    /// authoritative document back through it rather than trusting the
    /// controller's own projection.
    fn graph_controller_on_real_project_host(
        root: &std::path::Path,
    ) -> (EditorGraphController, ProjectHostClient, DocumentId) {
        let client = crate::project_host::test_project_host_client_at(root);
        let node_catalog = futures::executor::block_on(client.load_node_type_catalog())
            .expect("in-process project-host publishes a node type catalog");
        let graph_catalog = futures::executor::block_on(client.load_graph_type_catalog())
            .expect("in-process project-host publishes a graph type catalog");
        let document_id = DocumentId::new(QUEUE_DOCUMENT_ID);
        let snapshot = futures::executor::block_on(
            client.create_graph_document(&document_id, "az.editor.tests.logic-graph"),
        )
        .expect("create the graph document the queue mutates");
        assert_eq!(snapshot.revision, DocumentRevision::new(0));
        let adapter = EditorGraphUiAdapter::new(
            snapshot,
            node_catalog.clone(),
            GraphGeometrySnapshot::default(),
        )
        .expect("project-host returns a valid graph document");
        let mut controller = EditorGraphController::new_for_tests(
            client.clone(),
            node_catalog,
            graph_catalog,
            root.join("editor-side-channels"),
        );
        controller.current_document_id = Some(document_id.clone());
        controller.saved_revision = Some(DocumentRevision::new(0));
        controller.adapter = Some(adapter);
        (controller, client, document_id)
    }

    fn add_node_action(x: f32, y: f32) -> GraphControllerAction {
        GraphControllerAction::AddNode {
            node_type: QUEUE_NODE_TYPE.to_owned(),
            node_type_version: 1,
            layout: GraphNodeLayout { x, y },
        }
    }

    /// Appends an action the way the driver's own caller does, and returns the
    /// queue identity it was accepted under.
    fn accept(
        app: &mut App,
        fence: crate::controller_set::ControllerFence,
        action: GraphControllerAction,
    ) -> GraphActionQueueIdentity {
        match crate::controller_set::enqueue_graph_action(app, fence, action) {
            GraphActionAdmission::Start(identity) => identity,
            GraphActionAdmission::Queued => panic!("an unstarted queue must not report Queued"),
            GraphActionAdmission::Full => panic!("queue refused an action below its bound"),
            GraphActionAdmission::Retired => panic!("no live queue for a current graph fence"),
        }
    }

    fn host_document(
        client: &ProjectHostClient,
        document_id: &DocumentId,
    ) -> GraphDocumentSnapshot {
        futures::executor::block_on(client.graph_document_snapshot(document_id))
            .expect("read the authoritative document back from project-host")
    }

    fn installed_revision(app: &App) -> Option<DocumentRevision> {
        crate::controller_set::installed_graph_controller(app)
            .and_then(|controller| controller.adapter.map(|adapter| adapter.revision()))
    }

    /// The headline ordering contract: two mutations queued before either one
    /// starts reach project-host in invocation order at revisions R then R+1,
    /// and both survive.
    ///
    /// This is the real seam, not a stand-in: both batches cross
    /// `apply_graph_commands` into a live in-process `ProjectHost` that
    /// validates expected revisions. The pre-queue behaviour is what makes it
    /// discriminating -- two actions that both cloned the controller at
    /// revision 0 would both send `expected_revision: 0`, project-host would
    /// reject the second, and the document would end at revision 1 with one
    /// node. Passing requires the second action to have started from the
    /// controller the first one published.
    #[gpui::test]
    fn queued_graph_mutations_reach_project_host_in_order_at_successive_revisions(
        cx: &gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temp project root");
        let (controller, client, document_id) = graph_controller_on_real_project_host(root.path());

        cx.update(|app| {
            let fence = crate::controller_set::install_ready_graph_slot_for_tests(app, controller);
            // Both actions are queued before the driver starts. Under the test
            // RPC shim an action started at enqueue time runs to completion
            // inside that call, so enqueueing through the public entry points
            // would serialize them trivially and prove nothing about the queue.
            let identity = accept(app, fence, add_node_action(10.0, 20.0));
            let second = accept(app, fence, add_node_action(30.0, 40.0));
            assert_eq!(
                identity, second,
                "both actions belong to the same installed queue"
            );
            assert_eq!(crate::controller_set::pending_graph_action_count(app), 2);

            start_next_graph_action(app, fence, identity);

            assert_eq!(
                crate::controller_set::pending_graph_action_count(app),
                0,
                "the driver must drain the queue it started"
            );
        });

        let document = host_document(&client, &document_id);
        assert_eq!(
            document.revision,
            DocumentRevision::new(2),
            "each accepted mutation advanced the document exactly once"
        );
        let layouts: Vec<(f32, f32)> = document
            .document
            .nodes
            .iter()
            .map(|node| (node.layout.x, node.layout.y))
            .collect();
        assert_eq!(
            layouts,
            vec![(10.0, 20.0), (30.0, 40.0)],
            "project-host applied the batches in invocation order"
        );

        cx.update(|app| {
            assert_eq!(installed_revision(app), Some(DocumentRevision::new(2)));
            let projection = app
                .try_global::<EditorGraphDocumentProjection>()
                .expect("the last success published a projection");
            assert_eq!(
                projection
                    .document
                    .as_ref()
                    .expect("published projection carries the document")
                    .nodes
                    .len(),
                2
            );
            assert!(projection.status_error.is_none());
        });
    }

    /// A failing action publishes its typed error and changes nothing else: the
    /// controller that was installed before it stays installed.
    #[gpui::test]
    fn a_failed_graph_action_publishes_its_error_and_keeps_the_installed_controller(
        cx: &gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temp project root");
        let (controller, client, document_id) = graph_controller_on_real_project_host(root.path());

        cx.update(|app| {
            let fence = crate::controller_set::install_ready_graph_slot_for_tests(app, controller);
            let identity = accept(
                app,
                fence,
                GraphControllerAction::MoveComment {
                    comment_id: comment_id(0x99),
                    bounds: GraphCommentBounds {
                        x: 1.0,
                        y: 2.0,
                        width: 3.0,
                        height: 4.0,
                    },
                },
            );

            start_next_graph_action(app, fence, identity);

            assert_eq!(
                installed_revision(app),
                Some(DocumentRevision::new(0)),
                "a failure must not replace the last successful controller"
            );
            let projection = app
                .try_global::<EditorGraphDocumentProjection>()
                .expect("the failure published a projection error");
            let error = projection
                .status_error
                .as_deref()
                .expect("failed graph actions surface a status error");
            assert!(
                error.contains("unknown comment"),
                "expected the adapter's typed error, got {error}"
            );
        });

        assert_eq!(
            host_document(&client, &document_id).revision,
            DocumentRevision::new(0),
            "a locally rejected action must never reach project-host"
        );
    }

    /// Continuity: work queued behind a failure is neither dropped nor rebased.
    /// It runs from the controller that was installed when the failure landed.
    #[gpui::test]
    fn work_queued_behind_a_failed_action_still_runs_from_the_last_successful_controller(
        cx: &gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temp project root");
        let (controller, client, document_id) = graph_controller_on_real_project_host(root.path());

        cx.update(|app| {
            let fence = crate::controller_set::install_ready_graph_slot_for_tests(app, controller);
            let identity = accept(
                app,
                fence,
                GraphControllerAction::RemoveComment {
                    comment_id: comment_id(0x99),
                },
            );
            accept(app, fence, add_node_action(7.0, 9.0));

            start_next_graph_action(app, fence, identity);

            assert_eq!(crate::controller_set::pending_graph_action_count(app), 0);
            assert_eq!(installed_revision(app), Some(DocumentRevision::new(1)));
        });

        let document = host_document(&client, &document_id);
        assert_eq!(
            document.revision,
            DocumentRevision::new(1),
            "the surviving action planned against the pre-failure revision"
        );
        assert_eq!(document.document.nodes.len(), 1);
    }

    /// A completion whose queue was retired mid-flight publishes nothing and
    /// advances nothing. Retiring without reattaching keeps the controller
    /// fence current, which isolates the queue identity as the thing doing the
    /// fencing.
    #[gpui::test]
    fn a_completion_from_a_retired_queue_identity_cannot_publish_or_advance(
        cx: &gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temp project root");
        let (controller, _client, _document_id) =
            graph_controller_on_real_project_host(root.path());

        cx.update(|app| {
            let fence = crate::controller_set::install_ready_graph_slot_for_tests(app, controller);
            let identity = accept(app, fence, add_node_action(10.0, 20.0));
            accept(app, fence, add_node_action(30.0, 40.0));
            let (_started, mut in_flight) =
                crate::controller_set::start_next_graph_action(app, fence, identity)
                    .expect("the driver takes the first action");
            assert_eq!(crate::controller_set::pending_graph_action_count(app), 1);

            crate::controller_set::retire_graph_action_queue_for_tests(app);

            // The completion the retired action would have produced, carrying a
            // controller far ahead of the installed one.
            in_flight.saved_revision = Some(DocumentRevision::new(99));
            complete_graph_action_in_app(
                app,
                fence,
                identity,
                "failed to add graph node",
                Ok((in_flight, None)),
            );

            assert_eq!(
                crate::controller_set::installed_graph_controller(app)
                    .and_then(|controller| controller.saved_revision),
                Some(DocumentRevision::new(0)),
                "a retired completion must not replace the installed controller"
            );
            assert!(
                app.try_global::<EditorGraphDocumentProjection>().is_none(),
                "a retired completion must not publish a projection"
            );
            assert_eq!(
                crate::controller_set::pending_graph_action_count(app),
                0,
                "retiring the queue drops its pending work rather than advancing it"
            );
        });
    }

    /// Reattachment clears pending work and starts a new, empty identity, and
    /// the previous identity can no longer publish into it.
    #[gpui::test]
    fn reattachment_clears_pending_graph_work_and_installs_a_fresh_identity(
        cx: &gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temp project root");
        let (controller, _client, _document_id) =
            graph_controller_on_real_project_host(root.path());
        let reattached = controller.clone();

        cx.update(|app| {
            let fence = crate::controller_set::install_ready_graph_slot_for_tests(app, controller);
            let identity = accept(app, fence, add_node_action(10.0, 20.0));
            accept(app, fence, add_node_action(30.0, 40.0));
            let (_action, mut in_flight) =
                crate::controller_set::start_next_graph_action(app, fence, identity)
                    .expect("the driver takes the first action");

            let next_fence =
                crate::controller_set::install_ready_graph_slot_for_tests(app, reattached);

            assert_eq!(
                crate::controller_set::pending_graph_action_count(app),
                0,
                "reattachment clears pending graph work"
            );
            let next_identity = accept(app, next_fence, add_node_action(50.0, 60.0));
            assert_ne!(
                identity, next_identity,
                "the reattached session must not reuse the retired queue identity"
            );
            assert!(
                !crate::controller_set::graph_action_queue_is_current(app, fence, identity),
                "the retired identity is no longer current under its own fence"
            );

            in_flight.saved_revision = Some(DocumentRevision::new(99));
            complete_graph_action_in_app(
                app,
                fence,
                identity,
                "failed to add graph node",
                Ok((in_flight, None)),
            );

            assert_eq!(
                crate::controller_set::installed_graph_controller(app)
                    .and_then(|controller| controller.saved_revision),
                Some(DocumentRevision::new(0)),
                "a completion from the previous session cannot publish into the new one"
            );
            assert_eq!(
                crate::controller_set::pending_graph_action_count(app),
                1,
                "the new session's own pending action is untouched"
            );
        });
    }

    /// Only the two build-status operations write a console line, and both take
    /// it from the controller they produced. Every other operation stays silent,
    /// which is what keeps drag commits and saves out of the console.
    #[test]
    fn only_the_build_status_operations_report_to_the_console() {
        let mut controller = EditorGraphController::new_for_tests(
            crate::project_host::test_project_host_client(),
            NodeTypeCatalog::new(1, 1, Vec::new()),
            GraphTypeCatalog::new(1, 1, Vec::new()),
            std::env::temp_dir(),
        );
        controller.build_status = Some(GraphBuildStatusProjectionData {
            document_id: "graphs/console.visual.ron".to_owned(),
            source_path: "graphs/console.visual.ron".to_owned(),
            asset_guid: String::new(),
            source_status: GraphBuildSourceStatusData::Clean,
            entry_id: 1,
            content_hash: String::new(),
            latest_job: None,
        });
        let expected = graph_build_console_line(
            controller
                .build_status
                .as_ref()
                .expect("the controller carries a build status"),
        );

        for action in [
            GraphControllerAction::Build,
            GraphControllerAction::RefreshBuildStatus,
        ] {
            assert_eq!(
                action.success_log(&controller).as_deref(),
                Some(expected.as_str()),
                "`{}` must report the controller's build status",
                action.name()
            );
        }

        for action in [
            GraphControllerAction::Refresh,
            GraphControllerAction::Save,
            GraphControllerAction::AutoLayout,
            GraphControllerAction::RouteConnections,
            add_node_action(0.0, 0.0),
        ] {
            assert!(
                action.success_log(&controller).is_none(),
                "`{}` must not write to the console",
                action.name()
            );
        }
    }

    /// The bound is explicit: enqueueing past it returns the typed error and
    /// leaves every accepted action to run.
    #[gpui::test]
    fn the_graph_action_queue_refuses_work_past_its_bound_without_disturbing_accepted_work(
        cx: &gpui::TestAppContext,
    ) {
        let root = tempfile::tempdir().expect("temp project root");
        let (controller, client, document_id) = graph_controller_on_real_project_host(root.path());

        cx.update(|app| {
            let fence = crate::controller_set::install_ready_graph_slot_for_tests(app, controller);
            let identity = accept(app, fence, add_node_action(0.0, 0.0));
            for _ in 1..GRAPH_ACTION_QUEUE_CAPACITY {
                accept(app, fence, add_node_action(0.0, 0.0));
            }
            assert_eq!(
                crate::controller_set::pending_graph_action_count(app),
                GRAPH_ACTION_QUEUE_CAPACITY
            );

            let refused = refresh_graph_document(app)
                .expect_err("a full queue refuses new work through the public entry point");
            assert!(
                matches!(
                    refused,
                    EditorError::GraphActionQueueFull { capacity }
                        if capacity == GRAPH_ACTION_QUEUE_CAPACITY
                ),
                "expected the typed capacity error, got {refused:?}"
            );
            assert_eq!(
                crate::controller_set::pending_graph_action_count(app),
                GRAPH_ACTION_QUEUE_CAPACITY,
                "a refusal must not disturb work already accepted"
            );

            start_next_graph_action(app, fence, identity);

            assert_eq!(crate::controller_set::pending_graph_action_count(app), 0);
        });

        assert_eq!(
            host_document(&client, &document_id).document.nodes.len(),
            GRAPH_ACTION_QUEUE_CAPACITY,
            "every accepted action ran"
        );
    }
}
