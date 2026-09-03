//! Error types for the editor

use thiserror::Error;

/// Editor error types
#[derive(Error, Debug)]
pub enum EditorError {
    #[error("Editor service protocol error: {0}")]
    ServiceProtocol(#[from] capnp::Error),

    #[error("Editor RPC transport error: {0}")]
    RpcTransport(#[from] az_rpc::AzRpcTransportError),

    #[error("Editor RPC runtime error: {source}")]
    RpcRuntime {
        #[source]
        source: std::io::Error,
    },

    #[error("Service discovery error: {0}")]
    ServiceDiscovery(String),

    #[error("{service} returned an invalid {operation} result: {reason}")]
    ServiceAuthorityMismatch {
        service: &'static str,
        operation: &'static str,
        reason: String,
    },

    #[error(
        "no active session is available for editor attach; create one with `azoth session create <name>` or pass `--session <name>`"
    )]
    NoActiveEditorSession,

    #[error("multiple active sessions are available; pass `--session <name>` explicitly")]
    AmbiguousEditorSession,

    #[error("session `{session}` is {state}; editor attach requires an active session")]
    SessionNotActive { session: String, state: String },

    #[error(
        "session `{session}` has no registered {service} service; start/register project services before attaching the editor"
    )]
    MissingSessionService { session: String, service: String },

    #[error("session discovery mismatch during {operation} for `{session}`: {reason}")]
    SessionDiscoveryMismatch {
        operation: &'static str,
        session: String,
        reason: String,
    },

    #[error(
        "session `{session}` {service} service run {run} is {state}; editor attach requires it to be running"
    )]
    SessionServiceNotRunning {
        session: String,
        service: String,
        run: uuid::Uuid,
        state: String,
    },

    #[error("Project-host GameData catalog side-channel error: {0}")]
    ProjectHostGameDataCatalogSideChannel(
        #[from] az_proto_project::GameDataCatalogSideChannelError,
    ),

    #[error("Project-host node type catalog side-channel error: {0}")]
    ProjectHostNodeTypeCatalogSideChannel(
        #[from] az_proto_project::NodeTypeCatalogSideChannelError,
    ),

    #[error("Project-host graph type catalog side-channel error: {0}")]
    ProjectHostGraphTypeCatalogSideChannel(
        #[from] az_proto_project::GraphTypeCatalogSideChannelError,
    ),

    #[error("Project-host graph command status side-channel error: {0}")]
    ProjectHostGraphCommandStatusSideChannel(
        #[from] az_proto_project::GraphCommandStatusSideChannelError,
    ),

    #[error("Project-host graph document side-channel error: {0}")]
    ProjectHostGraphDocumentSideChannel(#[from] az_proto_project::GraphDocumentSideChannelError),

    #[error("Graph document creation error: {0}")]
    GraphDocumentCreation(#[from] crate::graph_ui::EditorGraphCreationError),

    #[error("Graph UI adapter error: {0}")]
    GraphUiAdapter(#[from] crate::graph_ui::EditorGraphUiAdapterError),

    #[error("failed to write graph command batch side-channel file {path}: {source}")]
    GraphCommandBatchSideChannelWrite {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("side-channel capability error: {0}")]
    SideChannelCapability(#[from] az_proto_core::SideChannelCapabilityError),

    #[error("Editor observability setup error: {0}")]
    Observability(#[from] az_observability::ObservedLogFileError),

    // Boxed: `ScaffoldError` is the largest payload reachable from this enum at
    // ~160 bytes, which pushed every `EditorResult` over `result_large_err`.
    // The `From` impl below keeps `?` on a `ScaffoldError` working unchanged.
    #[error("Project workflow error: {0}")]
    ProjectWorkflow(#[source] Box<az_project_scaffold::ScaffoldError>),

    #[error("failed to synchronize generated project targets: {0}")]
    GeneratedProjectTargets(String),

    #[error("Editor UI error: {0}")]
    Ui(#[from] az_editor_ui::Error),

    #[error("attached editor controller `{controller}` is still installing")]
    ControllerInstalling { controller: &'static str },

    #[error("attached editor controller `{controller}` failed: {message}")]
    ControllerFailed {
        controller: &'static str,
        message: String,
    },

    #[error("attached editor controller `{controller}` is unavailable for this session")]
    ControllerUnavailable { controller: &'static str },

    #[error(
        "the attached graph session already holds {capacity} pending graph actions; wait for queued graph work to finish before starting more"
    )]
    GraphActionQueueFull { capacity: usize },

    #[error(
        "asset source schema `{source_schema}` is editor-creatable through project document schema `{document_schema}`, but project-host did not publish that schema as a creatable authored document"
    )]
    CreatableSourceSchemaMissingProjectDocumentSchema {
        source_schema: String,
        document_schema: String,
    },

    #[error(
        "asset source schemas `{first_source_schema}` and `{second_source_schema}` both create project document schema `{document_schema}`; the editor creation action cannot disambiguate that workflow"
    )]
    AmbiguousCreatableSourceDocumentSchema {
        document_schema: String,
        first_source_schema: String,
        second_source_schema: String,
    },

    #[error("project document `{document_id}` was not found in the authored outline")]
    MissingProjectDocument { document_id: String },

    #[error("Reflected Prefab selection error: {0}")]
    ReflectedSelection(#[from] crate::authored_selection::ReflectedSelectionError),

    #[error("editor has no selected graph document; create or select a graph before editing it")]
    MissingGraphDocumentSelection,

    #[error("source navigation target `{target}` is unresolved")]
    SourceNavigationUnresolved { target: String },

    #[error("source navigation target `{target}` is docs-only but has no docs URL")]
    SourceNavigationMissingDocsUrl { target: String },

    #[error(
        "source navigation target `{target}` resolved as {path_kind} without a filesystem path"
    )]
    SourceNavigationMissingResolvedPath {
        target: String,
        path_kind: &'static str,
    },

    #[error("source navigation target `{target}` resolved to missing file `{path}`")]
    SourceNavigationMissingFile { target: String, path: String },

    #[error("invalid source navigation template `{template}`: {reason}")]
    InvalidSourceNavigationTemplate { template: String, reason: String },

    #[error("graph type `{graph_type}` was not published by the loaded project-host graph catalog")]
    GraphTypeNotPublished { graph_type: String },

    #[error(
        "node type `{node_type}` version {version} was not published by the loaded project-host node catalog"
    )]
    GraphNodeTypeNotPublished { node_type: String, version: u32 },

    #[error("graph node id `{node_id}` is invalid: {reason}")]
    InvalidGraphNodeId { node_id: String, reason: String },

    #[error("graph connection id `{connection_id}` is invalid: {reason}")]
    InvalidGraphConnectionId {
        connection_id: String,
        reason: String,
    },

    #[error("graph route anchor id `{anchor_id}` is invalid: {reason}")]
    InvalidGraphRouteAnchorId { anchor_id: String, reason: String },

    #[error("graph comment id `{comment_id}` is invalid: {reason}")]
    InvalidGraphCommentId { comment_id: String, reason: String },

    #[error("graph port id {port_id} is invalid: {reason}")]
    InvalidGraphPortId { port_id: u32, reason: String },

    #[error(
        "editor has no asset builder catalog; attach to a session with an asset-processor before creating asset source files"
    )]
    MissingAssetBuilderCatalog,

    #[error(
        "editor has no composed authored-document creation catalog; attach to a session with project-host schema catalog and asset-builder catalog before creating authored documents"
    )]
    MissingAuthoredDocumentCreationCatalog,

    #[error(
        "authored document schema `{schema}` was not published by the composed project-host/asset-builder creation catalog"
    )]
    AuthoredDocumentSchemaNotPublished { schema: String },

    #[error(
        "asset source file workflow for schema `{schema_type}` in source root `{source_root}` was not published by the loaded asset-builder catalog"
    )]
    AssetSourceWorkflowNotPublished {
        schema_type: String,
        source_root: String,
    },

    #[error("invalid asset source create request for `{source_path}`: {message}")]
    InvalidAssetSourceCreateRequest {
        source_path: String,
        message: String,
    },

    #[error(
        "runtime launch requires DB-owned asset source roots for session `{session}` asset view {workspace_id}; asset-processor returned none"
    )]
    MissingRuntimeAssetSourceRoots { session: String, workspace_id: i64 },

    #[error("editor has no attached session for {operation}")]
    MissingAttachedSession { operation: &'static str },

    #[error("invalid console command `{command}`: {message}")]
    InvalidConsoleCommand { command: String, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("GPUI error: {0}")]
    Gpui(String),

    #[error("Engine error: {0}")]
    Engine(String),

    #[error("Window error: {0}")]
    Window(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

/// Preserves `?` on a [`az_project_scaffold::ScaffoldError`] now that
/// [`EditorError::ProjectWorkflow`] boxes its payload; `#[from]` on the boxed
/// field would have produced `From<Box<ScaffoldError>>` instead.
impl From<az_project_scaffold::ScaffoldError> for EditorError {
    fn from(source: az_project_scaffold::ScaffoldError) -> Self {
        Self::ProjectWorkflow(Box::new(source))
    }
}

/// Convenient result type for editor operations
pub type EditorResult<T> = Result<T, EditorError>;
