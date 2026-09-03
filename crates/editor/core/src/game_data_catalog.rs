//! Editor-owned `GameData` catalog controller.
//!
//! Project-host owns project-linked `GameData` descriptor inventory. The editor
//! receives a serialized snapshot, joins it with the published schema catalog
//! and the loaded authored document, and publishes the read-only
//! [`az_editor_ui::panels::EditorGameDataProjection`] global that the `GameData`
//! mode panels render. Selection/navigation state lives here and is mutated by
//! the typed `GameData*` actions dispatched from the panels.

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use az_editor_inspector::{ReflectedScalar, ReflectedValue, decode_reflected_envelope};
use az_editor_ui::panels::{
    EditorGameDataProjection, EditorTypeRegistry, GameDataDiagnosticProjection,
    GameDataFieldDetailProjection, GameDataFieldKind, GameDataGridCellProjection,
    GameDataGridColumnProjection, GameDataGridProjection, GameDataGridRowProjection,
    GameDataGridRowState, GameDataInspectorProjection, GameDataInspectorTab,
    GameDataManagerCompositionProjection, GameDataManagerFilterRowProjection,
    GameDataManagerGroupKind, GameDataManagerGroupProjection, GameDataManagerNodeChipProjection,
    GameDataManagerProjectionRowProjection, GameDataManagerRowProjection,
    GameDataManagerSourceProjection, GameDataManagerStageProjection,
    GameDataManagerValidationRowProjection, GameDataRailView, GameDataSchemaEditorProjection,
    GameDataSchemaFieldProjection, GameDataSchemaRowProjection, GameDataSchemaUsageProjection,
    GameDataTableFolderProjection, GameDataTableRowProjection, GameDataTone,
    GameDataWorkbenchProjection,
};
use az_proto_asset::WorkspaceSourceFileRef;
use az_proto_project::vnext::{
    ReflectedFieldDescriptor, ReflectedTypeDescriptor, ReflectedTypeKind,
    SourceAuthoringSessionCommand, SourceAuthoringSessionOutcome, SourceFileEditOperation,
    SourceFileEditSnapshot, TypeRegistrySnapshot,
};
use az_proto_project::{
    GameDataCatalogDiagnostic, GameDataCatalogSnapshot, GameDataManagerCatalogEntry,
    GameDataManagerInput, GameDataProviderTarget, GameDataTableDescriptor,
};
use gpui::{App, Global};
use tracing::{error, info, instrument};

use crate::attach::EditorAttachSession;
use crate::error::{EditorError, EditorResult};
use crate::mode_projection::{
    ModeProjectionInputs, ModeProjectionRegistration, ModeProjectionRegistrationError,
    ModeProjectionSpec, publish_mode_projection_and_refresh,
};
use crate::project_host::ProjectHostClient;

#[derive(Debug, Clone)]
pub struct EditorGameDataCatalog {
    pub catalog: GameDataCatalogSnapshot,
}

impl EditorGameDataCatalog {
    #[must_use]
    pub const fn new(catalog: GameDataCatalogSnapshot) -> Self {
        Self { catalog }
    }
}

impl Global for EditorGameDataCatalog {}

#[derive(Clone)]
pub struct EditorGameDataCatalogController {
    catalog_session: Option<EditorAttachSession>,
    source_transport: Arc<dyn SourceAuthoringTransport>,
    /// Serializes the source-authoring lifecycle across the RPC workers.
    ///
    /// The document state itself is owned here, while UI presentation remains
    /// on the GPUI thread and is guarded separately by its request token.
    source_authoring: Arc<tokio::sync::Mutex<GameDataSourceAuthoring>>,
    source_request: Arc<AtomicU64>,
}

impl EditorGameDataCatalogController {
    /// Build a catalog controller bound to an already-attached session.
    ///
    /// # Errors
    ///
    /// Currently infallible — the controller only clones the session handle and
    /// allocates its source-authoring state. The `Result` is kept because
    /// callers handle it alongside the other controller constructors, which do
    /// open transports.
    pub fn connect_attached(session: &EditorAttachSession) -> EditorResult<Self> {
        Ok(Self {
            catalog_session: Some(session.clone()),
            source_transport: Arc::new(AttachedSourceAuthoringTransport {
                session: session.clone(),
            }),
            source_authoring: Arc::new(tokio::sync::Mutex::new(GameDataSourceAuthoring::default())),
            source_request: Arc::new(AtomicU64::new(0)),
        })
    }

    /// Reload the `GameData` catalog from project-host for the attached session.
    ///
    /// # Errors
    ///
    /// Returns [`EditorError::ServiceDiscovery`] if this controller was built
    /// without a session (the source-authoring test controller), or any error
    /// [`ProjectHostClient::connect_for_session`] or
    /// [`ProjectHostClient::load_gamedata_catalog`] returns — a failed
    /// project-host connection, a rejected capability, or a malformed catalog
    /// reply.
    #[instrument(skip_all)]
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    pub async fn refresh_catalog(&self) -> EditorResult<GameDataCatalogSnapshot> {
        let session = self.catalog_session.as_ref().ok_or_else(|| {
            EditorError::ServiceDiscovery(
                "GameData catalog refresh is unavailable in a source-authoring test controller"
                    .to_owned(),
            )
        })?;
        let project_host = ProjectHostClient::connect_for_session(
            &session.services.project_host,
            session.session_id,
        )
        .await?;
        let catalog = project_host.load_gamedata_catalog().await?;
        info!(
            tables = catalog.tables.len(),
            families = catalog.families.len(),
            managers = catalog.managers.len(),
            diagnostics = catalog.diagnostics.len(),
            "loaded GameData catalog from project-host"
        );
        Ok(catalog)
    }

    fn begin_source_request(&self, request: u64) {
        self.source_request.store(request, Ordering::Release);
    }

    fn source_request_is_current(&self, request: u64) -> bool {
        self.source_request.load(Ordering::Acquire) == request
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn open_table(
        &self,
        request: u64,
        source: WorkspaceSourceFileRef,
    ) -> EditorResult<(u64, u32, u32, SourceFileEditSnapshot)> {
        if !self.source_request_is_current(request) {
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData selection".to_owned(),
            ));
        }
        let mut lifecycle = self.source_authoring.lock().await;
        if !self.source_request_is_current(request) {
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData selection".to_owned(),
            ));
        }
        if let Some(active) = lifecycle
            .active
            .as_ref()
            .filter(|active| active.source == source)
        {
            return Ok((
                active.revision,
                active.undo_depth,
                active.redo_depth,
                active.snapshot.clone(),
            ));
        }
        let source_transport = Arc::clone(&self.source_transport);
        if let Some(active) = lifecycle.active.as_ref() {
            ensure_source_authoring_success(
                source_transport
                    .source_authoring_session(
                        active.source.clone(),
                        active.revision,
                        SourceAuthoringSessionCommand::Close,
                    )
                    .await?,
            )?;
            lifecycle.active = None;
        }
        if !self.source_request_is_current(request) {
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData selection".to_owned(),
            ));
        }
        let result = source_transport
            .source_authoring_session(source.clone(), 0, SourceAuthoringSessionCommand::Open)
            .await?;
        let revision = result.status.revision;
        let undo_depth = result.status.undo_depth;
        let redo_depth = result.status.redo_depth;
        let snapshot = ensure_source_authoring_snapshot(result)?;
        if !self.source_request_is_current(request) {
            let _ = source_transport
                .source_authoring_session(
                    source.clone(),
                    revision,
                    SourceAuthoringSessionCommand::Close,
                )
                .await;
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData selection".to_owned(),
            ));
        }
        lifecycle.active = Some(ActiveGameDataSource {
            source: source.clone(),
            revision,
            undo_depth,
            redo_depth,
            snapshot: snapshot.clone(),
        });
        drop(lifecycle);
        Ok((revision, undo_depth, redo_depth, snapshot))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn apply_source_operation(
        &self,
        request: u64,
        expected_source: WorkspaceSourceFileRef,
        command: SourceAuthoringSessionCommand,
    ) -> EditorResult<(u64, u32, u32, SourceFileEditSnapshot)> {
        let mut lifecycle = self.source_authoring.lock().await;
        if !self.source_request_is_current(request) {
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData operation".to_owned(),
            ));
        }
        let active = lifecycle.active.as_ref().ok_or_else(|| {
            EditorError::ServiceDiscovery("no active GameData source session".to_owned())
        })?;
        if active.source != expected_source {
            return Err(EditorError::ServiceDiscovery(
                "GameData operation does not target the active source".to_owned(),
            ));
        }
        let source = active.source.clone();
        let revision = active.revision;
        let source_transport = Arc::clone(&self.source_transport);
        let result = match source_transport
            .source_authoring_session(source.clone(), revision, command)
            .await
        {
            Ok(result) => result,
            Err(error) => {
                lifecycle.active = None;
                return Err(error);
            }
        };
        let revision = result.status.revision;
        let undo_depth = result.status.undo_depth;
        let redo_depth = result.status.redo_depth;
        let snapshot = match ensure_source_authoring_snapshot(result) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                discard_game_data_source_session(
                    &mut lifecycle,
                    source_transport.as_ref(),
                    &source,
                    revision,
                )
                .await;
                return Err(error);
            }
        };
        if !self.source_request_is_current(request) {
            discard_game_data_source_session(
                &mut lifecycle,
                source_transport.as_ref(),
                &source,
                revision,
            )
            .await;
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData operation".to_owned(),
            ));
        }
        lifecycle.active = Some(ActiveGameDataSource {
            source,
            revision,
            undo_depth,
            redo_depth,
            snapshot: snapshot.clone(),
        });
        drop(lifecycle);
        Ok((revision, undo_depth, redo_depth, snapshot))
    }

    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn close_source(
        &self,
        request: u64,
        expected_source: WorkspaceSourceFileRef,
    ) -> EditorResult<()> {
        let mut lifecycle = self.source_authoring.lock().await;
        if !self.source_request_is_current(request) {
            return Err(EditorError::ServiceDiscovery(
                "superseded GameData close".to_owned(),
            ));
        }
        let Some(active) = lifecycle.active.as_ref() else {
            return Ok(());
        };
        if active.source != expected_source {
            return Err(EditorError::ServiceDiscovery(
                "GameData close does not target the active source".to_owned(),
            ));
        }
        ensure_source_authoring_success(
            self.source_transport
                .source_authoring_session(
                    active.source.clone(),
                    active.revision,
                    SourceAuthoringSessionCommand::Close,
                )
                .await?,
        )?;
        lifecycle.active = None;
        drop(lifecycle);
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct ActiveGameDataSource {
    source: WorkspaceSourceFileRef,
    revision: u64,
    undo_depth: u32,
    redo_depth: u32,
    snapshot: SourceFileEditSnapshot,
}

#[derive(Default)]
struct GameDataSourceAuthoring {
    active: Option<ActiveGameDataSource>,
}

// The transport crosses from GPUI to the dedicated RPC thread, but each
// Cap'n Proto request is driven locally on that thread's `LocalSet`.
type SourceAuthoringFuture<'a> = Pin<
    Box<
        dyn Future<Output = EditorResult<az_proto_project::vnext::SourceAuthoringSessionResult>>
            + 'a,
    >,
>;

trait SourceAuthoringTransport: Send + Sync {
    fn source_authoring_session(
        &self,
        source: WorkspaceSourceFileRef,
        expected_revision: u64,
        command: SourceAuthoringSessionCommand,
    ) -> SourceAuthoringFuture<'_>;
}

#[derive(Clone)]
struct AttachedSourceAuthoringTransport {
    session: EditorAttachSession,
}

impl SourceAuthoringTransport for AttachedSourceAuthoringTransport {
    fn source_authoring_session(
        &self,
        source: WorkspaceSourceFileRef,
        expected_revision: u64,
        command: SourceAuthoringSessionCommand,
    ) -> SourceAuthoringFuture<'_> {
        Box::pin(async move {
            ProjectHostClient::connect_for_session(
                &self.session.services.project_host,
                self.session.session_id,
            )
            .await?
            .source_authoring_session(&source, expected_revision, command)
            .await
        })
    }
}

/// Retires the editor's speculative view before attempting remote cleanup.
///
/// A failed edit leaves the asset processor as the sole authority for the
/// document. Closing is intentionally best effort: the next selection must
/// reopen from that authority even when the failed transport cannot be reached.
#[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
async fn discard_game_data_source_session(
    lifecycle: &mut GameDataSourceAuthoring,
    source_transport: &dyn SourceAuthoringTransport,
    source: &WorkspaceSourceFileRef,
    revision: u64,
) {
    lifecycle.active = None;
    let _ = source_transport
        .source_authoring_session(
            source.clone(),
            revision,
            SourceAuthoringSessionCommand::Close,
        )
        .await;
}

fn ensure_source_authoring_success(
    result: az_proto_project::vnext::SourceAuthoringSessionResult,
) -> EditorResult<()> {
    match result.outcome {
        SourceAuthoringSessionOutcome::Failure(failure) => Err(EditorError::ServiceDiscovery(
            format!("source authoring {:?}: {}", failure.code, failure.detail),
        )),
        SourceAuthoringSessionOutcome::Snapshot(_) | SourceAuthoringSessionOutcome::Closed => {
            Ok(())
        }
    }
}

fn ensure_source_authoring_snapshot(
    result: az_proto_project::vnext::SourceAuthoringSessionResult,
) -> EditorResult<SourceFileEditSnapshot> {
    match result.outcome {
        SourceAuthoringSessionOutcome::Snapshot(snapshot) => Ok(snapshot),
        SourceAuthoringSessionOutcome::Failure(failure) => Err(EditorError::ServiceDiscovery(
            format!("source authoring {:?}: {}", failure.code, failure.detail),
        )),
        SourceAuthoringSessionOutcome::Closed => Err(EditorError::ServiceDiscovery(
            "source authoring closed without a canonical snapshot".to_owned(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Selection state
// ---------------------------------------------------------------------------

/// Which catalog entity kind currently drives the workbench and inspector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GameDataSelectionKind {
    #[default]
    Table,
    Schema,
    Manager,
}

/// Editor-core-owned `GameData` mode UI state. Inputs to the projection build;
/// never rendered directly.
#[derive(Debug, Clone, Default)]
struct GameDataUiState {
    rail_view: GameDataRailView,
    selected_kind: GameDataSelectionKind,
    selected_table_key: String,
    selected_schema_key: String,
    selected_manager_key: String,
    selected_field_key: Option<String>,
    inspector_tab: Option<GameDataInspectorTab>,
    grid_filter: String,
    source_session: GameDataSourceSession,
    source_request: u64,
}

/// The open authoring session for one `GameData` source file: which file it
/// edits, how deep its undo and redo stacks currently run, and the snapshot the
/// grid renders. The two depths are properties of this one open session and are
/// meaningless apart from the snapshot they were measured against.
#[derive(Debug, Clone)]
struct GameDataSourceReady {
    source: WorkspaceSourceFileRef,
    undo_depth: u32,
    redo_depth: u32,
    snapshot: SourceFileEditSnapshot,
}

#[derive(Debug, Clone, Default)]
enum GameDataSourceSession {
    #[default]
    None,
    Loading,
    /// Boxed: the snapshot carries the whole decoded source document, and would
    /// otherwise set the size of every `GameDataUiState` the editor holds,
    /// including the idle and error states.
    Ready(Box<GameDataSourceReady>),
    Error {
        source: WorkspaceSourceFileRef,
        detail: String,
    },
}

impl Global for GameDataUiState {}

struct GameDataMode;

impl ModeProjectionSpec for GameDataMode {
    type State = GameDataUiState;
    type Projection = EditorGameDataProjection;

    const NAME: &'static str = "gamedata";

    fn register_inputs(inputs: &mut ModeProjectionInputs) {
        inputs.depends_on::<EditorGameDataCatalog>();
        inputs.depends_on::<EditorTypeRegistry>();
    }

    fn install_actions(cx: &mut App) {
        install_game_data_actions(cx);
    }

    fn project(state: &Self::State, cx: &App) -> Self::Projection {
        let catalog = cx
            .try_global::<EditorGameDataCatalog>()
            .map(|catalog| &catalog.catalog);
        let schemas = cx
            .try_global::<EditorTypeRegistry>()
            .map(|registry| &registry.snapshot);
        build_game_data_projection(catalog, schemas, state)
    }
}

pub(crate) fn mode_projection_registration()
-> Result<ModeProjectionRegistration, ModeProjectionRegistrationError> {
    ModeProjectionRegistration::for_spec::<GameDataMode>()
}

fn install_game_data_actions(cx: &mut App) {
    install_game_data_selection_actions(cx);
    install_game_data_edit_actions(cx);
}

/// Catalog refresh plus the rail/table/schema/manager/field selection actions
/// that only move `GameDataUiState` and republish the projection.
fn install_game_data_selection_actions(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::RefreshGameDataCatalog, cx| {
        if let Err(err) = refresh_game_data_catalog(cx) {
            error!(error = %err, "failed to handle GameData catalog refresh action");
        }
    });

    cx.on_action(|action: &az_editor_ui::actions::SetGameDataRailView, cx| {
        let view = action.view;
        let state = cx.default_global::<GameDataUiState>();
        state.rail_view = view;
        state.selected_kind = match view {
            GameDataRailView::Tables => GameDataSelectionKind::Table,
            GameDataRailView::Schemas => GameDataSelectionKind::Schema,
            GameDataRailView::Managers => GameDataSelectionKind::Manager,
        };
        state.inspector_tab = None;
        publish_mode_projection_and_refresh::<GameDataMode>(cx);
    });

    cx.on_action(|action: &az_editor_ui::actions::SelectGameDataTable, cx| {
        let table_key = action.table_key.clone();
        let source = cx
            .try_global::<EditorGameDataCatalog>()
            .and_then(|catalog| table_by_key(&catalog.catalog, &table_key))
            .map(|table| table.source_ref.clone());
        let request = {
            let state = cx.default_global::<GameDataUiState>();
            state.source_request = state.source_request.saturating_add(1);
            state.source_request
        };
        {
            let state = cx.default_global::<GameDataUiState>();
            state.selected_table_key = table_key;
            state.selected_kind = GameDataSelectionKind::Table;
            state.rail_view = GameDataRailView::Tables;
            state.selected_field_key = None;
            state.inspector_tab = Some(GameDataInspectorTab::Table);
            state.grid_filter.clear();
            state.source_session = source.as_ref().map_or(GameDataSourceSession::None, |_| {
                GameDataSourceSession::Loading
            });
        }
        if let Some(source) = source {
            if let Ok(attached) = game_data_controller(cx) {
                attached.controller.begin_source_request(request);
            }
            open_game_data_table(cx, request, source);
        }
        publish_mode_projection_and_refresh::<GameDataMode>(cx);
    });

    cx.on_action(|action: &az_editor_ui::actions::SelectGameDataSchema, cx| {
        let state = cx.default_global::<GameDataUiState>();
        state.selected_schema_key.clone_from(&action.schema_key);
        state.selected_kind = GameDataSelectionKind::Schema;
        state.rail_view = GameDataRailView::Schemas;
        state.inspector_tab = Some(GameDataInspectorTab::Schema);
        publish_mode_projection_and_refresh::<GameDataMode>(cx);
    });

    cx.on_action(
        |action: &az_editor_ui::actions::SelectGameDataManager, cx| {
            let state = cx.default_global::<GameDataUiState>();
            state.selected_manager_key.clone_from(&action.manager_key);
            state.selected_kind = GameDataSelectionKind::Manager;
            state.rail_view = GameDataRailView::Managers;
            state.inspector_tab = Some(GameDataInspectorTab::Manager);
            publish_mode_projection_and_refresh::<GameDataMode>(cx);
        },
    );

    cx.on_action(|action: &az_editor_ui::actions::SelectGameDataField, cx| {
        let state = cx.default_global::<GameDataUiState>();
        state.selected_field_key = Some(action.field_key.clone());
        state.inspector_tab = Some(GameDataInspectorTab::Field);
        publish_mode_projection_and_refresh::<GameDataMode>(cx);
    });

    cx.on_action(
        |action: &az_editor_ui::actions::SetGameDataInspectorTab, cx| {
            let state = cx.default_global::<GameDataUiState>();
            state.inspector_tab = Some(action.tab);
            publish_mode_projection_and_refresh::<GameDataMode>(cx);
        },
    );

    cx.on_action(
        |action: &az_editor_ui::actions::SetGameDataGridFilter, cx| {
            let state = cx.default_global::<GameDataUiState>();
            state.grid_filter.clone_from(&action.filter);
            publish_mode_projection_and_refresh::<GameDataMode>(cx);
        },
    );
}

/// Row edits and the source-session lifecycle: every one of these dispatches a
/// source-authoring command against the active document.
fn install_game_data_edit_actions(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::AddGameDataRow, cx| {
        run_game_data_operation(cx, SourceFileEditOperation::AppendDefault);
    });
    cx.on_action(|action: &az_editor_ui::actions::DuplicateGameDataRow, cx| {
        run_game_data_operation(
            cx,
            SourceFileEditOperation::DuplicateObject {
                object_id: action.object_id.clone(),
            },
        );
    });
    cx.on_action(|action: &az_editor_ui::actions::RemoveGameDataRow, cx| {
        run_game_data_operation(
            cx,
            SourceFileEditOperation::RemoveObject {
                object_id: action.object_id.clone(),
            },
        );
    });
    cx.on_action(|_: &az_editor_ui::actions::UndoGameDataRows, cx| {
        run_game_data_session_command(cx, SourceAuthoringSessionCommand::Undo);
    });
    cx.on_action(|_: &az_editor_ui::actions::RedoGameDataRows, cx| {
        run_game_data_session_command(cx, SourceAuthoringSessionCommand::Redo);
    });
    cx.on_action(|_: &az_editor_ui::actions::CloseGameDataTable, cx| {
        close_game_data_table(cx);
    });
}

pub(crate) fn install_game_data_catalog_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: crate::controller_set::ControllerFence,
) {
    let session_slug = session.session_slug.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "gamedata-catalog-install",
        move || async move { EditorGameDataCatalogController::connect_attached(&session) },
        move |cx, result| match result {
            Ok(controller) => {
                if !crate::controller_set::complete_game_data(cx, fence, controller) {
                    return;
                }
                info!(session = %session_slug, "installed GameData catalog controller");
            }
            Err(err) => {
                crate::controller_set::fail_controller(cx, fence, err.to_string());
                error!(
                    error = %err,
                    session = %session_slug,
                    "failed to connect GameData catalog controller"
                );
            }
        },
    );
}

fn game_data_controller(
    cx: &App,
) -> EditorResult<crate::controller_set::AttachedController<EditorGameDataCatalogController>> {
    crate::controller_set::game_data_controller(cx)
}

fn open_game_data_table(cx: &mut App, request: u64, source: WorkspaceSourceFileRef) {
    let attached = match game_data_controller(cx) {
        Ok(controller) => controller,
        Err(error) => {
            set_game_data_source_error(cx, request, source, error.to_string());
            return;
        }
    };
    let fence = attached.fence;
    let controller = attached.controller;
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "gamedata-source-open",
        {
            let open_source = source.clone();
            move || async move { controller.open_table(request, open_source).await }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((_, undo_depth, redo_depth, snapshot)) => {
                    set_game_data_source_ready(cx, request, undo_depth, redo_depth, snapshot);
                }
                Err(error) => set_game_data_source_error(cx, request, source, error.to_string()),
            }
        },
    );
}

fn run_game_data_operation(cx: &mut App, operation: SourceFileEditOperation) {
    run_game_data_session_command(cx, SourceAuthoringSessionCommand::Apply(operation));
}

/// `GameData` owns editor-wide Undo whenever it is the active workspace mode.
/// It consumes an empty-history command rather than falling through to a
/// stale Prefab selection; Prefab owns every other workspace mode.
pub(crate) fn try_undo_active_game_data(cx: &mut App) -> bool {
    if !crate::ui_state_persistence::is_gamedata_mode(cx) {
        return false;
    }
    let can_undo = cx.try_global::<GameDataUiState>().is_some_and(|state| {
        matches!(
            &state.source_session,
            GameDataSourceSession::Ready(ready) if ready.undo_depth > 0
        )
    });
    if can_undo {
        run_game_data_session_command(cx, SourceAuthoringSessionCommand::Undo);
    }
    true
}

/// See [`try_undo_active_game_data`].
pub(crate) fn try_redo_active_game_data(cx: &mut App) -> bool {
    if !crate::ui_state_persistence::is_gamedata_mode(cx) {
        return false;
    }
    let can_redo = cx.try_global::<GameDataUiState>().is_some_and(|state| {
        matches!(
            &state.source_session,
            GameDataSourceSession::Ready(ready) if ready.redo_depth > 0
        )
    });
    if can_redo {
        run_game_data_session_command(cx, SourceAuthoringSessionCommand::Redo);
    }
    true
}

fn run_game_data_session_command(cx: &mut App, command: SourceAuthoringSessionCommand) {
    let Some((source, undo_depth, redo_depth)) =
        cx.try_global::<GameDataUiState>()
            .and_then(|state| match &state.source_session {
                GameDataSourceSession::Ready(ready) => {
                    Some((ready.source.clone(), ready.undo_depth, ready.redo_depth))
                }
                _ => None,
            })
    else {
        return;
    };
    if matches!(command, SourceAuthoringSessionCommand::Undo) && undo_depth == 0
        || matches!(command, SourceAuthoringSessionCommand::Redo) && redo_depth == 0
    {
        return;
    }
    let request = {
        let state = cx.default_global::<GameDataUiState>();
        state.source_request = state.source_request.saturating_add(1);
        state.source_request
    };
    let attached = match game_data_controller(cx) {
        Ok(controller) => controller,
        Err(error) => {
            set_game_data_source_error(cx, request, source, error.to_string());
            return;
        }
    };
    let fence = attached.fence;
    let controller = attached.controller;
    controller.begin_source_request(request);
    cx.default_global::<GameDataUiState>().source_session = GameDataSourceSession::Loading;
    publish_mode_projection_and_refresh::<GameDataMode>(cx);
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "gamedata-source-edit",
        {
            let expected_source = source.clone();
            move || async move {
                controller
                    .apply_source_operation(request, expected_source, command)
                    .await
            }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok((_, undo_depth, redo_depth, snapshot)) => {
                    set_game_data_source_ready(cx, request, undo_depth, redo_depth, snapshot);
                }
                Err(error) => set_game_data_source_error(cx, request, source, error.to_string()),
            }
        },
    );
}

fn close_game_data_table(cx: &mut App) {
    let Some(source) =
        cx.try_global::<GameDataUiState>()
            .and_then(|state| match &state.source_session {
                GameDataSourceSession::Ready(ready) => Some(ready.source.clone()),
                GameDataSourceSession::Error { source, .. } => Some(source.clone()),
                _ => None,
            })
    else {
        return;
    };
    let request = {
        let state = cx.default_global::<GameDataUiState>();
        state.source_request = state.source_request.saturating_add(1);
        state.source_request
    };
    let attached = match game_data_controller(cx) {
        Ok(controller) => controller,
        Err(error) => {
            set_game_data_source_error(cx, request, source, error.to_string());
            return;
        }
    };
    let fence = attached.fence;
    let controller = attached.controller;
    controller.begin_source_request(request);
    cx.default_global::<GameDataUiState>().source_session = GameDataSourceSession::Loading;
    publish_mode_projection_and_refresh::<GameDataMode>(cx);
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "gamedata-source-close",
        {
            let expected_source = source.clone();
            move || async move { controller.close_source(request, expected_source).await }
        },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(()) => {
                    if cx.default_global::<GameDataUiState>().source_request == request {
                        cx.default_global::<GameDataUiState>().source_session =
                            GameDataSourceSession::None;
                        publish_mode_projection_and_refresh::<GameDataMode>(cx);
                    }
                }
                Err(error) => set_game_data_source_error(cx, request, source, error.to_string()),
            }
        },
    );
}

fn set_game_data_source_ready(
    cx: &mut App,
    request: u64,
    undo_depth: u32,
    redo_depth: u32,
    snapshot: SourceFileEditSnapshot,
) {
    let selected = cx
        .try_global::<EditorGameDataCatalog>()
        .and_then(|catalog| {
            cx.try_global::<GameDataUiState>()
                .and_then(|state| table_by_key(&catalog.catalog, &state.selected_table_key))
        })
        .map(|table| table.source_ref.clone());
    if cx.default_global::<GameDataUiState>().source_request != request
        || selected.as_ref() != Some(&snapshot.source)
    {
        return;
    }
    cx.default_global::<GameDataUiState>().source_session =
        GameDataSourceSession::Ready(Box::new(GameDataSourceReady {
            source: snapshot.source.clone(),
            undo_depth,
            redo_depth,
            snapshot,
        }));
    publish_mode_projection_and_refresh::<GameDataMode>(cx);
}

fn set_game_data_source_error(
    cx: &mut App,
    request: u64,
    source: WorkspaceSourceFileRef,
    detail: String,
) {
    if cx.default_global::<GameDataUiState>().source_request != request {
        return;
    }
    cx.default_global::<GameDataUiState>().source_session =
        GameDataSourceSession::Error { source, detail };
    publish_mode_projection_and_refresh::<GameDataMode>(cx);
}

/// Spawn a background refresh of the `GameData` catalog against the attached
/// session's controller, publishing the result into the GPUI globals.
///
/// # Errors
///
/// Returns any error [`crate::controller_set::game_data_controller`] returns:
/// [`EditorError::ControllerInstalling`] if the `GameData` controller is still
/// installing, [`EditorError::ControllerFailed`] if its install failed, or
/// [`EditorError::ControllerUnavailable`] if this session has no `GameData`
/// controller. A failure of the spawned refresh itself is logged, not
/// returned.
pub fn refresh_game_data_catalog(cx: &mut App) -> EditorResult<()> {
    let attached = crate::controller_set::game_data_controller(cx)?;
    let fence = attached.fence;
    let controller = attached.controller;

    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "gamedata-catalog-refresh",
        move || async move { controller.refresh_catalog().await },
        move |cx, result| {
            if !crate::controller_set::is_current_fence(cx, fence) {
                return;
            }
            match result {
                Ok(catalog) => {
                    let manager_count = catalog.managers.len();
                    cx.set_global(EditorGameDataCatalog::new(catalog));
                    info!(manager_count, "refreshed GameData catalog");
                }
                Err(err) => {
                    error!(error = %err, "failed to refresh GameData catalog");
                }
            }
        },
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Projection build (pure)
// ---------------------------------------------------------------------------

const OBJECT_ID_COLUMN: &str = "object_id";
const OBJECT_ID_WIDTH: u32 = 158;
const MAX_GRID_COLUMNS: usize = 20;
const DESCRIPTOR_SOURCE_NOTE: &str = "Descriptor source rendering unavailable: project-host \
     publishes resolved descriptors, not registration source text.";

fn build_game_data_projection(
    catalog: Option<&GameDataCatalogSnapshot>,
    schemas: Option<&TypeRegistrySnapshot>,
    state: &GameDataUiState,
) -> EditorGameDataProjection {
    let Some(catalog) = catalog else {
        return EditorGameDataProjection::default();
    };

    let selected_table = selected_table(catalog, state);
    let selected_schema_key = selected_schema_key(catalog, state, selected_table);
    let selected_manager = selected_manager(catalog, state);

    let loaded_document_id = match &state.source_session {
        GameDataSourceSession::Ready(ready) => catalog
            .tables
            .iter()
            .find(|table| table.source_ref == ready.source)
            .map(|table| table.document_id.as_str()),
        _ => None,
    };
    let table_folders = table_folders(
        catalog,
        selected_table.map(|table| table.name.as_str()),
        loaded_document_id,
    );
    let schema_rows = schema_rows(catalog, schemas, selected_schema_key.as_deref());
    let manager_groups = manager_groups(
        catalog,
        selected_manager.map(|manager| manager.key.as_str()),
    );

    let workbench = match state.selected_kind {
        GameDataSelectionKind::Table => selected_table.map_or_else(
            || GameDataWorkbenchProjection::Empty {
                message: "No GameData tables registered by project or engine gems.".to_owned(),
            },
            |table| GameDataWorkbenchProjection::Grid(build_grid(catalog, schemas, state, table)),
        ),
        GameDataSelectionKind::Schema => selected_schema_key.as_deref().map_or_else(
            || GameDataWorkbenchProjection::Empty {
                message: "No row schemas referenced by GameData tables.".to_owned(),
            },
            |schema_key| {
                GameDataWorkbenchProjection::Schema(build_schema_editor(
                    catalog, schemas, schema_key,
                ))
            },
        ),
        GameDataSelectionKind::Manager => selected_manager.map_or_else(
            || GameDataWorkbenchProjection::Empty {
                message: "No manager descriptors in the resolved catalog.".to_owned(),
            },
            |manager| {
                GameDataWorkbenchProjection::Manager(build_manager_composition(catalog, manager))
            },
        ),
    };

    let inspector = build_inspector(state, &workbench);

    EditorGameDataProjection {
        catalog_loaded: true,
        rail_view: state.rail_view,
        table_count: catalog.tables.len(),
        schema_count: schema_rows.len(),
        manager_count: catalog.managers.len(),
        table_folders,
        schemas: schema_rows,
        manager_groups,
        workbench,
        inspector,
        diagnostics: catalog
            .diagnostics
            .iter()
            .map(diagnostic_projection)
            .collect(),
    }
}

fn build_inspector(
    state: &GameDataUiState,
    workbench: &GameDataWorkbenchProjection,
) -> GameDataInspectorProjection {
    let (tabs, default_tab) = match workbench {
        GameDataWorkbenchProjection::Grid(grid) => {
            let mut tabs = Vec::new();
            if grid.selected_field.is_some() {
                tabs.push(GameDataInspectorTab::Field);
            }
            tabs.push(GameDataInspectorTab::Table);
            (tabs, GameDataInspectorTab::Table)
        }
        GameDataWorkbenchProjection::Schema(_) => (
            vec![GameDataInspectorTab::Schema],
            GameDataInspectorTab::Schema,
        ),
        GameDataWorkbenchProjection::Manager(_) => (
            vec![GameDataInspectorTab::Manager],
            GameDataInspectorTab::Manager,
        ),
        GameDataWorkbenchProjection::Empty { .. } => (Vec::new(), GameDataInspectorTab::Table),
    };
    let tab = state
        .inspector_tab
        .filter(|tab| tabs.contains(tab))
        .unwrap_or(default_tab);
    GameDataInspectorProjection { tab, tabs }
}

// --- selection resolution ---

fn table_by_key<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    key: &str,
) -> Option<&'a GameDataTableDescriptor> {
    if key.trim().is_empty() {
        return None;
    }
    catalog.tables.iter().find(|table| {
        table.name == key
            || table.document_id == key
            || table.source_path == key
            || table.source_root == key
    })
}

fn table_by_family_key<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    key: &str,
) -> Option<&'a GameDataTableDescriptor> {
    catalog
        .families
        .iter()
        .find(|family| family.name == key)
        .and_then(|family| {
            family
                .tables
                .iter()
                .find_map(|table| table_by_key(catalog, table))
        })
}

fn table_by_provider_target<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    target: &GameDataProviderTarget,
) -> Option<&'a GameDataTableDescriptor> {
    match target.kind.as_str() {
        "table" => table_by_key(catalog, &target.name),
        "family" => table_by_family_key(catalog, &target.name),
        _ => None,
    }
}

fn table_by_manager_input<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    input: &GameDataManagerInput,
) -> Option<&'a GameDataTableDescriptor> {
    match input.kind.as_str() {
        "table" | "provider" => table_by_key(catalog, &input.name),
        "family" => table_by_family_key(catalog, &input.name),
        _ => None,
    }
}

fn table_by_manager_node_key<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    key: &str,
) -> Option<&'a GameDataTableDescriptor> {
    if let Some(table_name) = key.strip_prefix("provider:table:") {
        return table_by_key(catalog, table_name);
    }
    if let Some(family_name) = key.strip_prefix("provider:family:") {
        return table_by_family_key(catalog, family_name);
    }
    None
}

fn first_table_for_manager<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    manager: &GameDataManagerCatalogEntry,
) -> Option<&'a GameDataTableDescriptor> {
    manager
        .provider_target
        .as_ref()
        .and_then(|target| table_by_provider_target(catalog, target))
        .or_else(|| {
            manager
                .source_targets
                .iter()
                .find_map(|target| table_by_provider_target(catalog, target))
        })
        .or_else(|| {
            manager
                .inputs
                .iter()
                .find_map(|input| table_by_manager_input(catalog, input))
        })
        .or_else(|| {
            manager
                .dependencies
                .iter()
                .find_map(|node| table_by_manager_node_key(catalog, &node.key))
        })
}

fn selected_table<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    state: &GameDataUiState,
) -> Option<&'a GameDataTableDescriptor> {
    table_by_key(catalog, &state.selected_table_key).or_else(|| catalog.tables.first())
}

fn selected_schema_key(
    catalog: &GameDataCatalogSnapshot,
    state: &GameDataUiState,
    selected_table: Option<&GameDataTableDescriptor>,
) -> Option<String> {
    if !state.selected_schema_key.is_empty() {
        return Some(state.selected_schema_key.clone());
    }
    selected_table
        .map(schema_key_for_table)
        .or_else(|| catalog.tables.first().map(schema_key_for_table))
}

fn selected_manager<'a>(
    catalog: &'a GameDataCatalogSnapshot,
    state: &GameDataUiState,
) -> Option<&'a GameDataManagerCatalogEntry> {
    if !state.selected_manager_key.is_empty()
        && let Some(manager) = catalog.manager(&state.selected_manager_key)
    {
        return Some(manager);
    }
    catalog.managers.first()
}

fn schema_key_for_table(table: &GameDataTableDescriptor) -> String {
    non_empty_string_or(&table.schema_type, &table.row_type)
}

fn schema_for_key<'a>(
    schemas: &'a TypeRegistrySnapshot,
    key: &str,
) -> Option<&'a ReflectedTypeDescriptor> {
    schemas.types.iter().find(|schema| {
        schema.type_path == key
            || schema.short_path == key
            || schema.type_path.rsplit(['.', ':']).next() == Some(key)
            || schema.short_path.rsplit([':', '.']).next() == Some(key)
    })
}

// --- rails ---

fn table_folders(
    catalog: &GameDataCatalogSnapshot,
    selected_table: Option<&str>,
    loaded_document_id: Option<&str>,
) -> Vec<GameDataTableFolderProjection> {
    let mut by_category = BTreeMap::<String, Vec<&GameDataTableDescriptor>>::new();
    for table in &catalog.tables {
        by_category
            .entry(non_empty_string_or(&table.category, "Project"))
            .or_default()
            .push(table);
    }

    by_category
        .into_iter()
        .map(|(category, mut tables)| {
            tables.sort_by(|left, right| left.name.cmp(&right.name));
            GameDataTableFolderProjection {
                name: category,
                tables: tables
                    .into_iter()
                    .map(|table| {
                        let loaded = loaded_document_id == Some(table.document_id.as_str());
                        GameDataTableRowProjection {
                            key: table.name.clone(),
                            name: table.name.clone(),
                            document_id: table.document_id.clone(),
                            count_label: table.row_count.map_or_else(
                                || {
                                    if loaded {
                                        "loaded".to_owned()
                                    } else {
                                        "catalog".to_owned()
                                    }
                                },
                                |count| count.to_string(),
                            ),
                            selected: selected_table == Some(table.name.as_str()),
                            loaded,
                        }
                    })
                    .collect(),
            }
        })
        .collect()
}

fn schema_rows(
    catalog: &GameDataCatalogSnapshot,
    schemas: Option<&TypeRegistrySnapshot>,
    selected_schema: Option<&str>,
) -> Vec<GameDataSchemaRowProjection> {
    let mut used_by = BTreeMap::<String, usize>::new();
    for table in &catalog.tables {
        *used_by.entry(schema_key_for_table(table)).or_default() += 1;
    }

    used_by
        .into_iter()
        .map(|(schema_key, table_count)| {
            let schema = schemas.and_then(|schemas| schema_for_key(schemas, &schema_key));
            GameDataSchemaRowProjection {
                label: schema.map_or_else(
                    || kind_label(&schema_key),
                    |schema| {
                        schema
                            .editor_attributes
                            .label
                            .clone()
                            .unwrap_or_else(|| kind_label(&schema.short_path))
                    },
                ),
                used_by_label: plural_count(table_count, "table"),
                selected: selected_schema == Some(schema_key.as_str()),
                resolved: schema.is_some(),
                key: schema_key,
            }
        })
        .collect()
}

fn manager_groups(
    catalog: &GameDataCatalogSnapshot,
    selected_key: Option<&str>,
) -> Vec<GameDataManagerGroupProjection> {
    let mut auto = Vec::new();
    let mut by_owner = BTreeMap::<String, Vec<&GameDataManagerCatalogEntry>>::new();
    for manager in &catalog.managers {
        if manager.read_only {
            auto.push(manager);
        } else {
            by_owner
                .entry(non_empty_string_or(&manager.owner, "unowned descriptors"))
                .or_default()
                .push(manager);
        }
    }

    let manager_row = |manager: &GameDataManagerCatalogEntry| GameDataManagerRowProjection {
        key: manager.key.clone(),
        name: manager.name.clone(),
        kind_label: if manager.read_only {
            "Auto"
        } else {
            "Authored"
        }
        .to_owned(),
        read_only: manager.read_only,
        has_diagnostics: !manager.diagnostics.is_empty(),
        selected: selected_key == Some(manager.key.as_str()),
    };

    let mut groups = Vec::new();
    if !auto.is_empty() {
        auto.sort_by(|left, right| left.name.cmp(&right.name));
        groups.push(GameDataManagerGroupProjection {
            key: "automatic-providers".to_owned(),
            name: "Automatic providers".to_owned(),
            kind: GameDataManagerGroupKind::AutomaticProviders,
            managers: auto.into_iter().map(manager_row).collect(),
        });
    }
    for (owner, mut managers) in by_owner {
        managers.sort_by(|left, right| left.name.cmp(&right.name));
        groups.push(GameDataManagerGroupProjection {
            key: owner.clone(),
            name: owner,
            kind: GameDataManagerGroupKind::OwnerGem,
            managers: managers.into_iter().map(manager_row).collect(),
        });
    }
    groups
}

// --- grid ---

/// Grid contents derived from the source-authoring session backing one table:
/// a loaded document, a failed open, or a load still in flight.
struct GameDataGridContents {
    columns: Vec<GameDataGridColumnProjection>,
    rows: Vec<GameDataGridRowProjection>,
    total_row_count: usize,
    row_state: GameDataGridRowState,
    can_edit: bool,
    can_undo: bool,
    can_redo: bool,
}

/// Only a loaded document yields rows and an editable grid; the other two
/// states render the schema's columns with an empty, read-only body.
fn grid_contents_for_source_session(
    schemas: Option<&TypeRegistrySnapshot>,
    state: &GameDataUiState,
    schema_key: &str,
    source_session: Option<(&SourceFileEditSnapshot, u32, u32)>,
    source_failed: bool,
) -> GameDataGridContents {
    let Some((snapshot, undo_depth, redo_depth)) = source_session else {
        let columns = schemas
            .and_then(|schemas| schema_for_key(schemas, schema_key))
            .map(columns_from_schema)
            .unwrap_or_default();
        return GameDataGridContents {
            columns,
            rows: Vec::new(),
            total_row_count: 0,
            row_state: if source_failed {
                GameDataGridRowState::Error
            } else {
                GameDataGridRowState::Loading
            },
            can_edit: false,
            can_undo: false,
            can_redo: false,
        };
    };

    let columns = schemas
        .and_then(|schemas| schema_for_key(schemas, schema_key))
        .map_or_else(|| vec![object_id_column()], columns_from_schema);
    let all_rows = rows_from_source_document(snapshot, schemas, &columns);
    let total_row_count = all_rows.len();
    let filter = state.grid_filter.trim().to_ascii_lowercase();
    let rows = if filter.is_empty() {
        all_rows
    } else {
        all_rows
            .into_iter()
            .filter(|row| row_matches_filter(row, &filter))
            .collect()
    };
    GameDataGridContents {
        columns,
        rows,
        total_row_count,
        row_state: GameDataGridRowState::Loaded,
        can_edit: true,
        can_undo: undo_depth > 0,
        can_redo: redo_depth > 0,
    }
}

fn build_grid(
    catalog: &GameDataCatalogSnapshot,
    schemas: Option<&TypeRegistrySnapshot>,
    state: &GameDataUiState,
    table: &GameDataTableDescriptor,
) -> GameDataGridProjection {
    let schema_key = schema_key_for_table(table);
    let source_session = match &state.source_session {
        GameDataSourceSession::Ready(ready) if ready.source == table.source_ref => {
            Some((&ready.snapshot, ready.undo_depth, ready.redo_depth))
        }
        _ => None,
    };
    let source_error = match &state.source_session {
        GameDataSourceSession::Error { source, detail } if source == &table.source_ref => {
            Some(detail.clone())
        }
        _ => None,
    };
    let source_failed = source_error.is_some();

    let GameDataGridContents {
        columns,
        rows,
        total_row_count,
        row_state,
        can_edit,
        can_undo,
        can_redo,
    } = grid_contents_for_source_session(
        schemas,
        state,
        &schema_key,
        source_session,
        source_failed,
    );

    let selected_field = state.selected_field_key.as_deref().and_then(|field_key| {
        field_detail(
            catalog,
            schemas,
            source_session.map(|(snapshot, _, _)| snapshot),
            &schema_key,
            &columns,
            field_key,
        )
    });

    let mut columns = columns;
    for column in &mut columns {
        column.selected = state.selected_field_key.as_deref() == Some(column.key.as_str());
    }

    GameDataGridProjection {
        table_key: table.name.clone(),
        table_name: table.name.clone(),
        document_id: table.document_id.clone(),
        source_path: table.source_path.clone(),
        owner: table.owner.clone(),
        category: non_empty_string_or(&table.category, "Project"),
        families: table.families.clone(),
        schema_label: kind_label(&schema_key),
        schema_key,
        primary_key_label: OBJECT_ID_COLUMN.to_owned(),
        row_state,
        can_edit,
        can_undo,
        can_redo,
        status_detail: source_error,
        columns,
        rows,
        total_row_count,
        filter: state.grid_filter.clone(),
        selected_field,
        diagnostics: catalog
            .diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.target_key == table.name || diagnostic.target_key == table.document_id
            })
            .map(diagnostic_projection)
            .collect(),
    }
}

fn columns_from_schema(schema: &ReflectedTypeDescriptor) -> Vec<GameDataGridColumnProjection> {
    let mut columns = vec![object_id_column()];
    if schema.kind == ReflectedTypeKind::Struct {
        columns.extend(
            schema
                .fields
                .iter()
                .filter(|field| !field.editor_attributes.hidden)
                .take(MAX_GRID_COLUMNS)
                .map(|field| {
                    let (kind, type_label) = schema_field_kind(field);
                    GameDataGridColumnProjection {
                        key: field.name.clone(),
                        label: non_empty_string_or(
                            field.editor_attributes.label.as_deref().unwrap_or_default(),
                            &field.name,
                        ),
                        width: schema_field_width(field, &type_label),
                        type_label,
                        kind,
                        primary: false,
                        selected: false,
                    }
                }),
        );
    }
    columns
}

fn object_id_column() -> GameDataGridColumnProjection {
    GameDataGridColumnProjection {
        key: OBJECT_ID_COLUMN.to_owned(),
        label: "Object".to_owned(),
        type_label: "Key".to_owned(),
        kind: GameDataFieldKind::Key,
        width: OBJECT_ID_WIDTH,
        primary: true,
        selected: false,
    }
}

/// Project the worker-owned canonical document directly. `GameData` never falls
/// back to the Prefab inspection/session path: the codec's object ids are the
/// row identities used by duplicate/remove operations.
fn rows_from_source_document(
    snapshot: &SourceFileEditSnapshot,
    schemas: Option<&TypeRegistrySnapshot>,
    columns: &[GameDataGridColumnProjection],
) -> Vec<GameDataGridRowProjection> {
    snapshot
        .document
        .objects
        .iter()
        .enumerate()
        .map(|(index, object)| {
            let values = schemas
                .and_then(|schemas| decode_reflected_envelope(schemas, &object.value).ok())
                .and_then(|value| match value {
                    ReflectedValue::Struct(values) => Some(values),
                    _ => None,
                })
                .unwrap_or_default();
            let mut cells = vec![GameDataGridCellProjection {
                column_key: OBJECT_ID_COLUMN.to_owned(),
                text: object.object_id.clone(),
                kind: GameDataFieldKind::Key,
                bool_value: None,
            }];
            cells.extend(
                columns
                    .iter()
                    .filter(|column| !column.primary)
                    .map(|column| {
                        let value = values
                            .iter()
                            .find(|(name, _)| name == &column.key)
                            .map(|(_, value)| value);
                        GameDataGridCellProjection {
                            column_key: column.key.clone(),
                            text: source_value_label(value),
                            kind: column.kind,
                            bool_value: value.and_then(source_value_bool),
                        }
                    }),
            );
            GameDataGridRowProjection {
                object_id: object.object_id.clone(),
                index_label: (index + 1).to_string(),
                cells,
            }
        })
        .collect()
}

const fn source_value_bool(value: &ReflectedValue) -> Option<bool> {
    match value {
        ReflectedValue::Scalar(ReflectedScalar::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn source_value_label(value: Option<&ReflectedValue>) -> String {
    match value {
        Some(ReflectedValue::Scalar(ReflectedScalar::Bool(value))) => value.to_string(),
        Some(
            ReflectedValue::Scalar(
                ReflectedScalar::Signed(value)
                | ReflectedScalar::Unsigned(value)
                | ReflectedScalar::Float(value)
                | ReflectedScalar::String(value),
            )
            | ReflectedValue::OpaqueRon(value),
        ) => value.clone(),
        Some(ReflectedValue::Unit) => "()".to_owned(),
        Some(ReflectedValue::Optional(None)) => "null".to_owned(),
        Some(value) => format!("{value:?}"),
        None => String::new(),
    }
}

fn row_matches_filter(row: &GameDataGridRowProjection, filter: &str) -> bool {
    row.object_id.to_ascii_lowercase().contains(filter)
        || row
            .cells
            .iter()
            .any(|cell| cell.text.to_ascii_lowercase().contains(filter))
}

fn field_detail(
    catalog: &GameDataCatalogSnapshot,
    schemas: Option<&TypeRegistrySnapshot>,
    snapshot: Option<&SourceFileEditSnapshot>,
    schema_key: &str,
    columns: &[GameDataGridColumnProjection],
    field_key: &str,
) -> Option<GameDataFieldDetailProjection> {
    let column = columns.iter().find(|column| column.key == field_key)?;
    let shared_table_count = catalog
        .tables
        .iter()
        .filter(|table| schema_key_for_table(table) == schema_key)
        .count();

    let schema_field = schemas
        .and_then(|schemas| schema_for_key(schemas, schema_key))
        .filter(|schema| schema.kind == ReflectedTypeKind::Struct)
        .and_then(|schema| schema.fields.iter().find(|field| field.name == field_key));
    let value_preview = snapshot
        .and_then(|snapshot| snapshot.document.objects.first())
        .and_then(|object| {
            schemas
                .and_then(|schemas| decode_reflected_envelope(schemas, &object.value).ok())
                .and_then(|value| match value {
                    ReflectedValue::Struct(values) => values
                        .iter()
                        .find(|(name, _)| name == field_key)
                        .map(|(_, value)| source_value_label(Some(value))),
                    _ => None,
                })
        });

    Some(GameDataFieldDetailProjection {
        key: column.key.clone(),
        label: column.label.clone(),
        type_label: column.type_label.clone(),
        kind: column.kind,
        schema_label: kind_label(schema_key),
        shared_table_count,
        value_preview,
        description: schema_field.and_then(|field| field.editor_attributes.description.clone()),
        read_only: schema_field.is_some_and(|field| field.editor_attributes.read_only),
    })
}

// --- schema editor ---

fn build_schema_editor(
    catalog: &GameDataCatalogSnapshot,
    schemas: Option<&TypeRegistrySnapshot>,
    schema_key: &str,
) -> GameDataSchemaEditorProjection {
    let schema = schemas.and_then(|schemas| schema_for_key(schemas, schema_key));
    GameDataSchemaEditorProjection {
        schema_key: schema_key.to_owned(),
        label: schema.map_or_else(
            || kind_label(schema_key),
            |schema| {
                schema
                    .editor_attributes
                    .label
                    .clone()
                    .unwrap_or_else(|| kind_label(&schema.short_path))
            },
        ),
        rust_type: schema.map(|schema| schema.type_path.clone()),
        description: schema.and_then(|schema| schema.editor_attributes.description.clone()),
        resolved: schema.is_some(),
        fields: schema
            .map(|schema| {
                if schema.kind != ReflectedTypeKind::Struct {
                    return Vec::new();
                }
                schema
                    .fields
                    .iter()
                    .filter(|field| !field.editor_attributes.hidden)
                    .map(|field| {
                        let (kind, type_label) = schema_field_kind(field);
                        GameDataSchemaFieldProjection {
                            key: field.name.clone(),
                            label: non_empty_string_or(
                                field.editor_attributes.label.as_deref().unwrap_or_default(),
                                &field.name,
                            ),
                            type_label,
                            kind,
                            read_only: field.editor_attributes.read_only,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default(),
        used_by: schema_used_by(catalog, schema_key),
    }
}

fn schema_used_by(
    catalog: &GameDataCatalogSnapshot,
    schema_key: &str,
) -> Vec<GameDataSchemaUsageProjection> {
    catalog
        .tables
        .iter()
        .filter(|table| table.schema_type == schema_key || table.row_type == schema_key)
        .map(|table| GameDataSchemaUsageProjection {
            table_key: table.name.clone(),
            name: table.name.clone(),
            category: non_empty_string_or(&table.category, "Project"),
            count_label: table
                .row_count
                .map_or_else(|| "catalog".to_owned(), |count| plural_count(count, "row")),
        })
        .collect()
}

// --- manager composition ---

fn build_manager_composition(
    catalog: &GameDataCatalogSnapshot,
    manager: &GameDataManagerCatalogEntry,
) -> GameDataManagerCompositionProjection {
    let backing_table = first_table_for_manager(catalog, manager);
    let status_ok = manager.diagnostics.is_empty();
    let kind_label_text = if manager.read_only {
        "Auto"
    } else {
        "Authored"
    };
    let source_label = manager_source_label(manager);

    let summary = if manager.read_only {
        format!(
            "Auto-generated read-only provider over {source_label}; maintained by the resolved \
             catalog for every registered table/family."
        )
    } else {
        format!(
            "Authored manager descriptor owned by {} projecting {} rows into {}.",
            non_empty_string_or(&manager.owner, "unowned descriptors"),
            manager.row_type,
            manager.output_type
        )
    };

    GameDataManagerCompositionProjection {
        key: manager.key.clone(),
        name: manager.name.clone(),
        owner: non_empty_string_or(&manager.owner, "unowned descriptors"),
        kind_label: kind_label_text.to_owned(),
        catalog_kind: manager.kind.clone(),
        row_type: manager.row_type.clone(),
        output_type: manager.output_type.clone(),
        read_only: manager.read_only,
        summary,
        status_ok,
        status_label: if status_ok {
            "resolved".to_owned()
        } else {
            plural_count(manager.diagnostics.len(), "diagnostic")
        },
        backing_table_key: backing_table.map(|table| table.name.clone()),
        backing_table_name: backing_table.map(|table| table.name.clone()),
        stages: manager_stages(manager),
        sources: manager_sources(catalog, manager),
        projection_rows: manager_projection_rows(manager),
        projection_count: manager_projection_count(manager),
        consumers: manager.dependents.iter().map(node_chip).collect(),
        dependencies: manager.dependencies.iter().map(node_chip).collect(),
        key_policy_summary: key_policy_summary(manager),
        key_kind: manager.key_policy.kind.clone(),
        transform_chips: transform_chips(manager),
        duplicate_key_policy: manager.duplicate_key_policy.clone(),
        row_filters: manager
            .row_filters
            .iter()
            .map(|filter| GameDataManagerFilterRowProjection {
                field: filter.field.clone(),
                predicate: filter.predicate.clone(),
                compare_field: filter.compare_field.clone(),
            })
            .collect(),
        validation: manager_validation(manager),
        descriptor_note: DESCRIPTOR_SOURCE_NOTE.to_owned(),
    }
}

fn manager_stages(manager: &GameDataManagerCatalogEntry) -> Vec<GameDataManagerStageProjection> {
    let filter_count = manager.row_filters.len();
    let projection_count = manager.projection_transforms.len();
    vec![
        GameDataManagerStageProjection {
            key: "sources".to_owned(),
            title: "Sources".to_owned(),
            detail: format!("{} declared", manager.inputs.len()),
            tone: GameDataTone::Info,
        },
        GameDataManagerStageProjection {
            key: "filters".to_owned(),
            title: "Filters".to_owned(),
            detail: if filter_count == 0 {
                "none".to_owned()
            } else {
                filter_count.to_string()
            },
            tone: GameDataTone::Warning,
        },
        GameDataManagerStageProjection {
            key: "keys".to_owned(),
            title: "Key lowering".to_owned(),
            detail: manager.key_policy.kind.clone(),
            tone: GameDataTone::Accent,
        },
        GameDataManagerStageProjection {
            key: "projection".to_owned(),
            title: "Projection".to_owned(),
            detail: if projection_count == 0 {
                "implicit row projection".to_owned()
            } else {
                projection_count.to_string()
            },
            tone: GameDataTone::Success,
        },
        GameDataManagerStageProjection {
            key: "output".to_owned(),
            title: "Output".to_owned(),
            detail: manager.output_type.clone(),
            tone: GameDataTone::Neutral,
        },
    ]
}

fn manager_sources(
    catalog: &GameDataCatalogSnapshot,
    manager: &GameDataManagerCatalogEntry,
) -> Vec<GameDataManagerSourceProjection> {
    manager
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let dependency = manager.dependencies.iter().find(|dependency| {
                dependency.label == input.name
                    || dependency.key == input.name
                    || dependency
                        .key
                        .strip_prefix("manager:")
                        .is_some_and(|name| name == input.name)
                    || dependency
                        .key
                        .strip_prefix("provider:table:")
                        .is_some_and(|name| name == input.name)
                    || dependency
                        .key
                        .strip_prefix("provider:family:")
                        .is_some_and(|name| name == input.name)
            });
            let table = table_by_manager_input(catalog, input).or_else(|| {
                dependency.and_then(|node| table_by_manager_node_key(catalog, &node.key))
            });
            let manager_key = dependency
                .filter(|node| catalog.manager(&node.key).is_some())
                .map(|node| node.key.clone());
            GameDataManagerSourceProjection {
                key: dependency.map_or_else(
                    || format!("{}:{}:{}", input.kind, input.name, index),
                    |node| node.key.clone(),
                ),
                name: input.name.clone(),
                kind: input.kind.clone(),
                detail: non_empty_string_or(&input.detail, &input.row_type),
                table_key: table.map(|table| table.name.clone()),
                manager_key,
            }
        })
        .collect()
}

fn manager_projection_rows(
    manager: &GameDataManagerCatalogEntry,
) -> Vec<GameDataManagerProjectionRowProjection> {
    if manager.projection_transforms.is_empty() {
        return manager
            .source_targets
            .iter()
            .map(|target| GameDataManagerProjectionRowProjection {
                output_field: manager.output_type.clone(),
                source_label: format!("{} {}", target.kind, target.name),
                computed: false,
            })
            .collect();
    }
    manager
        .projection_transforms
        .iter()
        .map(|transform| GameDataManagerProjectionRowProjection {
            output_field: transform.field.clone(),
            source_label: if transform.source_column.is_empty() {
                transform.kind.clone()
            } else {
                format!("{} · {}", transform.source_column, transform.kind)
            },
            computed: transform.kind != "identity",
        })
        .collect()
}

fn manager_projection_count(manager: &GameDataManagerCatalogEntry) -> usize {
    if manager.projection_transforms.is_empty() {
        manager.source_targets.len().max(1)
    } else {
        manager.projection_transforms.len()
    }
}

fn node_chip(node: &az_proto_project::GameDataManagerNodeRef) -> GameDataManagerNodeChipProjection {
    GameDataManagerNodeChipProjection {
        key: node.key.clone(),
        label: node.label.clone(),
        kind: node.kind.clone(),
        provider: node.kind == "provider",
    }
}

fn key_policy_summary(manager: &GameDataManagerCatalogEntry) -> String {
    let mut parts = Vec::with_capacity(3 + manager.key_policy.transforms.len());
    parts.push(manager.key_policy.kind.clone());
    parts.extend(manager.key_policy.transforms.iter().cloned());
    if manager.key_policy.reject_zero_crc {
        parts.push("reject zero CRC".to_owned());
    }
    if manager.key_policy.store_key_text {
        parts.push("store key text".to_owned());
    }
    parts.join(" · ")
}

fn transform_chips(manager: &GameDataManagerCatalogEntry) -> Vec<String> {
    let mut labels = manager
        .key_policy
        .transforms
        .iter()
        .cloned()
        .chain(
            manager
                .projection_transforms
                .iter()
                .map(|transform| transform.kind.clone()),
        )
        .collect::<BTreeSet<_>>();
    if labels.is_empty() {
        labels.insert("identity".to_owned());
    }
    labels.into_iter().collect()
}

fn manager_validation(
    manager: &GameDataManagerCatalogEntry,
) -> Vec<GameDataManagerValidationRowProjection> {
    if manager.diagnostics.is_empty() {
        return vec![GameDataManagerValidationRowProjection {
            message: "descriptor graph resolved without catalog diagnostics".to_owned(),
            target_label: String::new(),
            ok: true,
        }];
    }
    manager
        .diagnostics
        .iter()
        .map(|diagnostic| GameDataManagerValidationRowProjection {
            message: diagnostic.message.clone(),
            target_label: diagnostic.target_label.clone(),
            ok: false,
        })
        .collect()
}

fn manager_source_label(manager: &GameDataManagerCatalogEntry) -> String {
    manager
        .inputs
        .first()
        .map(|input| non_empty_string_or(&input.name, &input.kind))
        .or_else(|| {
            manager
                .source_targets
                .first()
                .map(|target| non_empty_string_or(&target.name, &target.kind))
        })
        .unwrap_or_else(|| "no declared source".to_owned())
}

fn diagnostic_projection(diagnostic: &GameDataCatalogDiagnostic) -> GameDataDiagnosticProjection {
    GameDataDiagnosticProjection {
        code: diagnostic.code.clone(),
        message: diagnostic.message.clone(),
        target_label: diagnostic.target_label.clone(),
    }
}

// --- field classification (ported from the retired inline aether view) ---

fn schema_field_kind(field: &ReflectedFieldDescriptor) -> (GameDataFieldKind, String) {
    let schema_type = field.type_path.to_ascii_lowercase();
    if schema_type.contains("bool") {
        (GameDataFieldKind::Boolean, "Boolean".to_owned())
    } else if schema_type.contains("u8")
        || schema_type.contains("u16")
        || schema_type.contains("u32")
        || schema_type.contains("u64")
    {
        (GameDataFieldKind::Number, "Unsigned".to_owned())
    } else if schema_type.contains("i8")
        || schema_type.contains("i16")
        || schema_type.contains("i32")
        || schema_type.contains("i64")
    {
        (GameDataFieldKind::Number, "Integer".to_owned())
    } else if schema_type.contains("f32") || schema_type.contains("f64") {
        (GameDataFieldKind::Number, "Float".to_owned())
    } else if schema_type.contains("asset") {
        (GameDataFieldKind::Asset, "Asset".to_owned())
    } else if schema_type.contains("ref") || schema_type.contains("handle") {
        (GameDataFieldKind::Reference, "Ref".to_owned())
    } else if schema_type.contains("string") || schema_type.contains("str") {
        (GameDataFieldKind::Text, "String".to_owned())
    } else {
        (GameDataFieldKind::Object, kind_label(&field.type_path))
    }
}

fn schema_field_width(field: &ReflectedFieldDescriptor, type_label: &str) -> u32 {
    match type_label {
        "Boolean" => 82,
        "Unsigned" | "Integer" | "Float" => 92,
        "Asset" | "Ref" => 154,
        "String" if field.name.len() > 20 => 180,
        _ => 132,
    }
}

fn kind_label(schema_type: &str) -> String {
    let name = schema_type
        .rsplit([':', '.', '/', '\\'])
        .next()
        .unwrap_or(schema_type);
    let mut label = String::new();
    for (index, ch) in name.chars().enumerate() {
        if index > 0 && ch.is_uppercase() {
            label.push(' ');
        }
        label.push(ch);
    }
    non_empty_string_or(label, schema_type)
}

fn non_empty_string_or(preferred: impl AsRef<str>, fallback: &str) -> String {
    let preferred = preferred.as_ref().trim();
    if preferred.is_empty() {
        fallback.to_owned()
    } else {
        preferred.to_owned()
    }
}

fn plural_count(count: impl std::fmt::Display, noun: &str) -> String {
    let count = count.to_string();
    if count == "1" {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;
    use az_core::reflect::ReflectedValueEnvelope;
    use az_proto_asset::{SourceFileEditDocument, SourceFileEditObject};
    use az_proto_project::vnext::{
        ApplicabilityDescriptor, EditorAttributes, SourceAuthoringFailure,
        SourceAuthoringFailureCode, SourceAuthoringSessionResult,
    };
    use az_proto_project::{
        GAMEDATA_CATALOG_SNAPSHOT_VERSION, GameDataKeyPolicy, GameDataManagerNodeRef,
        GameDataProjectionTransform, GameDataTableFamilyDescriptor,
    };

    #[derive(Default)]
    struct RecordingSourceAuthoringTransport {
        responses: Mutex<VecDeque<SourceAuthoringSessionResult>>,
        calls: Mutex<Vec<(WorkspaceSourceFileRef, u64, SourceAuthoringSessionCommand)>>,
    }

    impl RecordingSourceAuthoringTransport {
        fn with_responses(
            responses: impl IntoIterator<Item = SourceAuthoringSessionResult>,
        ) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().collect()),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl SourceAuthoringTransport for RecordingSourceAuthoringTransport {
        fn source_authoring_session(
            &self,
            source: WorkspaceSourceFileRef,
            expected_revision: u64,
            command: SourceAuthoringSessionCommand,
        ) -> SourceAuthoringFuture<'_> {
            self.calls
                .lock()
                .expect("record source-authoring call")
                .push((source, expected_revision, command));
            let result = self
                .responses
                .lock()
                .expect("take source-authoring response")
                .pop_front()
                .expect("a response for every source-authoring call");
            Box::pin(std::future::ready(Ok(result)))
        }
    }

    fn source_authoring_controller(
        source_transport: Arc<dyn SourceAuthoringTransport>,
    ) -> EditorGameDataCatalogController {
        EditorGameDataCatalogController {
            catalog_session: None,
            source_transport,
            source_authoring: Arc::new(tokio::sync::Mutex::new(GameDataSourceAuthoring::default())),
            source_request: Arc::new(AtomicU64::new(0)),
        }
    }

    fn authoring_snapshot_result(
        snapshot: SourceFileEditSnapshot,
        revision: u64,
    ) -> SourceAuthoringSessionResult {
        SourceAuthoringSessionResult {
            status: az_proto_project::vnext::SourceAuthoringSessionStatus {
                open: true,
                revision,
                undo_depth: 0,
                redo_depth: 0,
            },
            outcome: SourceAuthoringSessionOutcome::Snapshot(snapshot),
        }
    }

    fn authoring_closed_result(revision: u64) -> SourceAuthoringSessionResult {
        SourceAuthoringSessionResult {
            status: az_proto_project::vnext::SourceAuthoringSessionStatus {
                open: false,
                revision,
                undo_depth: 0,
                redo_depth: 0,
            },
            outcome: SourceAuthoringSessionOutcome::Closed,
        }
    }

    fn catalog_fixture() -> GameDataCatalogSnapshot {
        GameDataCatalogSnapshot::new(
            GAMEDATA_CATALOG_SNAPSHOT_VERSION,
            1,
            vec![
                table_fixture(
                    "Items",
                    "ItemRow",
                    "sample::ItemRow",
                    "gamedata/items.ron",
                    "items.ron",
                    "items",
                    Some(2),
                    vec!["ItemFamily".to_owned()],
                ),
                table_fixture(
                    "ItemsClient",
                    "ItemRow",
                    "sample::ItemRow",
                    "gamedata/items_client.ron",
                    "items_client.ron",
                    "items",
                    None,
                    vec!["ItemFamily".to_owned()],
                ),
                table_fixture(
                    "Progression",
                    "ProgressionRow",
                    "sample::ProgressionRow",
                    "gamedata/progression.ron",
                    "progression.ron",
                    "progression",
                    Some(1),
                    Vec::new(),
                ),
            ],
            vec![GameDataTableFamilyDescriptor {
                name: "ItemFamily".to_owned(),
                row_type: "ItemRow".to_owned(),
                owner: "sample-gamedata".to_owned(),
                duplicate_key_policy: "overwrite".to_owned(),
                tables: vec!["Items".to_owned(), "ItemsClient".to_owned()],
            }],
            vec![manager_fixture()],
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn table_fixture(
        name: &str,
        row_type: &str,
        schema_type: &str,
        document_id: &str,
        source_path: &str,
        category: &str,
        row_count: Option<u64>,
        families: Vec<String>,
    ) -> GameDataTableDescriptor {
        GameDataTableDescriptor {
            name: name.to_owned(),
            row_type: row_type.to_owned(),
            source_root: "gamedata".to_owned(),
            source_path: source_path.to_owned(),
            owner: "sample-gamedata".to_owned(),
            schema_hash: Some(42),
            document_id: document_id.to_owned(),
            schema_type: schema_type.to_owned(),
            category: category.to_owned(),
            row_count,
            families,
            source_ref: WorkspaceSourceFileRef {
                source_root_key: "project:sample:assets".to_owned(),
                source_path: source_path.to_owned(),
                schema_type: "azoth.gamedata.TableSource".to_owned(),
            },
        }
    }

    fn manager_fixture() -> GameDataManagerCatalogEntry {
        GameDataManagerCatalogEntry {
            key: "manager:item-records".to_owned(),
            name: "Item records".to_owned(),
            owner: "sample-gamedata".to_owned(),
            row_type: "sample_game::ItemRecord".to_owned(),
            kind: "table manager".to_owned(),
            output_type: "sample_game::ItemRecordManager".to_owned(),
            read_only: false,
            provider_target: None,
            key_policy: GameDataKeyPolicy {
                kind: "CRC32".to_owned(),
                transforms: vec!["trim".to_owned(), "lowercase".to_owned()],
                reject_zero_crc: true,
                store_key_text: false,
            },
            duplicate_key_policy: "error".to_owned(),
            inputs: vec![
                GameDataManagerInput {
                    kind: "table".to_owned(),
                    name: "Items".to_owned(),
                    row_type: "sample_game::ItemRow".to_owned(),
                    source_root: "gamedata".to_owned(),
                    source_path: "items.ron".to_owned(),
                    detail: "primary item rows".to_owned(),
                    provider_kind: String::new(),
                },
                GameDataManagerInput {
                    kind: "manager".to_owned(),
                    name: "Localization provider".to_owned(),
                    row_type: "sample_game::LocalizedString".to_owned(),
                    source_root: String::new(),
                    source_path: String::new(),
                    detail: "display-name dependency".to_owned(),
                    provider_kind: "provider".to_owned(),
                },
            ],
            row_filters: Vec::new(),
            projection_transforms: vec![
                GameDataProjectionTransform {
                    field: "item_id".to_owned(),
                    source_column: "item_id".to_owned(),
                    kind: "identity".to_owned(),
                },
                GameDataProjectionTransform {
                    field: "display_name_crc".to_owned(),
                    source_column: "display_name".to_owned(),
                    kind: "lowercase crc string".to_owned(),
                },
            ],
            secondary_indexes: Vec::new(),
            source_targets: vec![GameDataProviderTarget {
                kind: "table".to_owned(),
                name: "Items".to_owned(),
                row_type: "sample_game::ItemRow".to_owned(),
            }],
            dependencies: vec![GameDataManagerNodeRef {
                key: "manager:Localization provider".to_owned(),
                label: "Localization provider".to_owned(),
                kind: "authored".to_owned(),
            }],
            dependents: Vec::new(),
            diagnostics: Vec::new(),
            projection_hash: Vec::new(),
        }
    }

    fn schema_catalog_fixture() -> TypeRegistrySnapshot {
        TypeRegistrySnapshot {
            schema_catalog_hash: vec![1; 32],
            types: vec![
                reflected_type(
                    "alloc::string::String",
                    "String",
                    ReflectedTypeKind::String,
                    vec![],
                ),
                reflected_type("bool", "bool", ReflectedTypeKind::Bool, vec![]),
                reflected_type(
                    "u32",
                    "u32",
                    ReflectedTypeKind::UnsignedInteger { bits: 32 },
                    vec![],
                ),
                reflected_type(
                    "sample::ItemRow",
                    "ItemRow",
                    ReflectedTypeKind::Struct,
                    vec![
                        reflected_field("item_id", "alloc::string::String"),
                        reflected_field("display_name", "alloc::string::String"),
                        reflected_field("enabled", "bool"),
                    ],
                ),
                reflected_type(
                    "sample::ProgressionRow",
                    "ProgressionRow",
                    ReflectedTypeKind::Struct,
                    vec![reflected_field("level", "u32")],
                ),
                reflected_type(
                    "alloc::vec::Vec<sample::ItemRow>",
                    "Vec<ItemRow>",
                    ReflectedTypeKind::List,
                    vec![],
                ),
                reflected_type(
                    "sample::ItemTable",
                    "ItemTable",
                    ReflectedTypeKind::Struct,
                    vec![reflected_field("rows", "alloc::vec::Vec<sample::ItemRow>")],
                ),
            ],
        }
    }

    fn reflected_type(
        type_path: &str,
        short_path: &str,
        kind: ReflectedTypeKind,
        fields: Vec<ReflectedFieldDescriptor>,
    ) -> ReflectedTypeDescriptor {
        ReflectedTypeDescriptor {
            type_path: type_path.to_owned(),
            short_path: short_path.to_owned(),
            kind,
            fields,
            variants: Vec::new(),
            editor_attributes: EditorAttributes {
                description: (short_path == "ItemRow")
                    .then(|| "Editable item table row.".to_owned()),
                ..EditorAttributes::default()
            },
            type_data_flags: Vec::new(),
            applicability: ApplicabilityDescriptor::default(),
            reflected_default: None,
        }
    }

    fn reflected_field(name: &str, type_path: &str) -> ReflectedFieldDescriptor {
        ReflectedFieldDescriptor {
            name: name.to_owned(),
            type_path: type_path.to_owned(),
            editor_attributes: EditorAttributes {
                label: Some(name.replace('_', " ")),
                ..EditorAttributes::default()
            },
        }
    }

    fn state_with(f: impl FnOnce(&mut GameDataUiState)) -> GameDataUiState {
        let mut state = GameDataUiState::default();
        f(&mut state);
        state
    }

    fn source_snapshot(table: &GameDataTableDescriptor) -> SourceFileEditSnapshot {
        let row = |object_id: &str, item_id: &str, display_name: &str, enabled: bool| {
            SourceFileEditObject {
                object_id: object_id.to_owned(),
                schema: "sample::ItemRow".to_owned(),
                value: ReflectedValueEnvelope::typed_ron(
                    "sample::ItemRow",
                    format!(
                        "(item_id:{item_id:?},display_name:{display_name:?},enabled:{enabled})"
                    ),
                ),
            }
        };
        SourceFileEditSnapshot {
            source: table.source_ref.clone(),
            source_fingerprint: vec![1],
            document: SourceFileEditDocument {
                root_object_id: None,
                root_schema: table.source_ref.schema_type.clone(),
                value: ReflectedValueEnvelope::typed_ron(&table.source_ref.schema_type, "()"),
                objects: vec![
                    row("iron-sword", "iron-sword", "Iron Sword", true),
                    row("iron-axe", "iron-axe", "Iron Axe", false),
                ],
                codec_state: Vec::new(),
            },
        }
    }

    fn ready_state(table: &GameDataTableDescriptor) -> GameDataUiState {
        state_with(|state| {
            state.source_session = GameDataSourceSession::Ready(Box::new(GameDataSourceReady {
                source: table.source_ref.clone(),
                undo_depth: 1,
                redo_depth: 0,
                snapshot: source_snapshot(table),
            }));
        })
    }

    #[test]
    fn transaction_conflict_discards_cached_session_and_reopens_same_table() {
        let catalog = catalog_fixture();
        let table = selected_table(&catalog, &GameDataUiState::default()).expect("table");
        let source = table.source_ref.clone();
        let initial = source_snapshot(table);
        let mut canonical = source_snapshot(table);
        canonical.source_fingerprint = vec![2];
        let transport = Arc::new(RecordingSourceAuthoringTransport::with_responses([
            authoring_snapshot_result(initial, 1),
            SourceAuthoringSessionResult {
                status: az_proto_project::vnext::SourceAuthoringSessionStatus {
                    open: true,
                    revision: 1,
                    undo_depth: 0,
                    redo_depth: 0,
                },
                outcome: SourceAuthoringSessionOutcome::Failure(SourceAuthoringFailure {
                    code: SourceAuthoringFailureCode::Transaction,
                    detail: "source fingerprint changed".to_owned(),
                    expected_revision: 1,
                    current_revision: 1,
                }),
            },
            authoring_closed_result(1),
            authoring_snapshot_result(canonical.clone(), 1),
        ]));
        let controller = source_authoring_controller(transport.clone());

        controller.begin_source_request(1);
        futures::executor::block_on(controller.open_table(1, source.clone()))
            .expect("open initial source");
        let error = futures::executor::block_on(controller.apply_source_operation(
            1,
            source.clone(),
            SourceAuthoringSessionCommand::Apply(SourceFileEditOperation::AppendDefault),
        ))
        .expect_err("external source change rejects cached transaction");
        assert!(error.to_string().contains("source fingerprint changed"));

        // Error-state teardown is idempotent after the failed transaction has
        // already retired the local session.
        futures::executor::block_on(controller.close_source(1, source.clone()))
            .expect("close remains available after conflict");
        controller.begin_source_request(2);
        let (_, _, _, reloaded) = futures::executor::block_on(controller.open_table(2, source))
            .expect("same table reopens from asset-processor authority");
        assert_eq!(reloaded.source_fingerprint, canonical.source_fingerprint);

        let calls = transport.calls.lock().expect("recorded calls");
        assert_eq!(calls.len(), 4);
        assert!(matches!(calls[0].2, SourceAuthoringSessionCommand::Open));
        assert!(matches!(
            calls[1].2,
            SourceAuthoringSessionCommand::Apply(_)
        ));
        assert!(matches!(calls[2].2, SourceAuthoringSessionCommand::Close));
        assert!(matches!(calls[3].2, SourceAuthoringSessionCommand::Open));
        drop(calls);
    }

    #[test]
    fn source_authoring_controller_opens_a_selected_catalog_table() {
        let catalog = catalog_fixture();
        let table = catalog.tables[0].clone();
        let snapshot = source_snapshot(&table);
        let transport = Arc::new(RecordingSourceAuthoringTransport::with_responses([
            authoring_snapshot_result(snapshot, 1),
        ]));
        let controller = source_authoring_controller(transport.clone());
        controller.begin_source_request(1);
        let (_, undo_depth, redo_depth, opened) =
            futures::executor::block_on(controller.open_table(1, table.source_ref.clone()))
                .expect("open selected table");

        assert_eq!(opened.source, table.source_ref);
        assert_eq!(undo_depth, 0);
        assert_eq!(redo_depth, 0);
        let calls = transport
            .calls
            .lock()
            .expect("recorded source-authoring calls");
        assert_eq!(calls.len(), 1);
        assert!(matches!(calls[0].2, SourceAuthoringSessionCommand::Open));
        drop(calls);
    }

    #[test]
    fn table_rail_groups_by_category_with_selection_and_load_state() {
        let catalog = catalog_fixture();
        let folders = table_folders(&catalog, Some("Items"), Some("gamedata/items.ron"));

        assert_eq!(
            folders
                .iter()
                .map(|folder| folder.name.as_str())
                .collect::<Vec<_>>(),
            vec!["items", "progression"]
        );
        let items = &folders[0].tables[0];
        assert_eq!(items.key, "Items");
        assert_eq!(items.document_id, "gamedata/items.ron");
        assert!(items.selected);
        assert!(items.loaded);
        assert_eq!(items.count_label, "2");
        // Unloaded table without a catalog row count reports catalog-only.
        assert_eq!(folders[0].tables[1].count_label, "catalog");
    }

    #[test]
    fn selection_falls_back_to_first_table_and_its_schema() {
        let catalog = catalog_fixture();
        let state = GameDataUiState::default();

        let table = selected_table(&catalog, &state).expect("default table");
        assert_eq!(table.name, "Items");
        assert_eq!(
            selected_schema_key(&catalog, &state, Some(table)).as_deref(),
            Some("sample::ItemRow")
        );

        let state =
            state_with(|state| state.selected_table_key = "gamedata/progression.ron".into());
        let table = selected_table(&catalog, &state).expect("table by document id");
        assert_eq!(table.name, "Progression");
    }

    #[test]
    fn grid_projects_loaded_document_rows_and_filter() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();
        let table = selected_table(&catalog, &GameDataUiState::default()).expect("table");
        let state = ready_state(table);

        let grid = build_grid(&catalog, Some(&schemas), &state, table);
        assert_eq!(grid.row_state, GameDataGridRowState::Loaded);
        assert!(grid.can_edit);
        assert!(grid.can_undo);
        assert!(!grid.can_redo);
        assert_eq!(
            grid.columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            vec!["object_id", "item_id", "display_name", "enabled"]
        );
        assert!(grid.columns[0].primary);
        assert_eq!(grid.columns[3].kind, GameDataFieldKind::Boolean);
        assert_eq!(grid.rows.len(), 2);
        assert_eq!(grid.total_row_count, 2);
        assert_eq!(grid.rows[0].object_id, "iron-sword");
        let enabled = grid.rows[0]
            .cells
            .iter()
            .find(|cell| cell.column_key == "enabled")
            .expect("enabled cell");
        assert_eq!(enabled.bool_value, Some(true));

        let mut state = ready_state(table);
        state.grid_filter = "axe".into();
        let grid = build_grid(&catalog, Some(&schemas), &state, table);
        assert_eq!(grid.total_row_count, 2);
        assert_eq!(grid.rows.len(), 1);
        assert_eq!(grid.rows[0].object_id, "iron-axe");
    }

    #[test]
    fn grid_without_loaded_document_projects_schema_columns_as_loading() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();
        let state = GameDataUiState::default();
        let table = selected_table(&catalog, &state).expect("table");

        let grid = build_grid(&catalog, Some(&schemas), &state, table);
        assert_eq!(grid.row_state, GameDataGridRowState::Loading);
        assert!(grid.rows.is_empty());
        assert_eq!(
            grid.columns
                .iter()
                .map(|column| column.key.as_str())
                .collect::<Vec<_>>(),
            vec!["object_id", "item_id", "display_name", "enabled"]
        );
    }

    #[test]
    fn grid_error_exposes_detail_without_stale_rows() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();
        let table = selected_table(&catalog, &GameDataUiState::default()).expect("table");
        let state = state_with(|state| {
            state.source_session = GameDataSourceSession::Error {
                source: table.source_ref.clone(),
                detail: "source changed on disk".to_owned(),
            };
        });

        let grid = build_grid(&catalog, Some(&schemas), &state, table);
        assert_eq!(grid.row_state, GameDataGridRowState::Error);
        assert_eq!(
            grid.status_detail.as_deref(),
            Some("source changed on disk")
        );
        assert!(grid.rows.is_empty());
        assert!(!grid.can_edit);
        assert!(!grid.can_undo);
        assert!(!grid.can_redo);
    }

    #[test]
    fn ready_snapshot_for_another_source_projects_loading_without_stale_rows() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();
        let loaded_table = &catalog.tables[0];
        let selected_table = &catalog.tables[1];
        let state = ready_state(loaded_table);

        let grid = build_grid(&catalog, Some(&schemas), &state, selected_table);
        assert_eq!(grid.row_state, GameDataGridRowState::Loading);
        assert!(grid.rows.is_empty());
        assert!(!grid.can_edit);
        assert!(!grid.can_undo);
        assert!(!grid.can_redo);
    }

    #[test]
    fn grid_selected_field_produces_field_detail() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();
        let table = selected_table(&catalog, &GameDataUiState::default()).expect("table");
        let mut state = ready_state(table);
        state.selected_field_key = Some("display_name".into());

        let grid = build_grid(&catalog, Some(&schemas), &state, table);
        let field = grid.selected_field.expect("selected field detail");
        assert_eq!(field.key, "display_name");
        assert_eq!(field.type_label, "String");
        assert_eq!(field.schema_label, "Item Row");
        assert_eq!(field.shared_table_count, 2);
        assert_eq!(field.value_preview.as_deref(), Some("Iron Sword"));
        assert!(
            grid.columns
                .iter()
                .find(|column| column.key == "display_name")
                .is_some_and(|column| column.selected)
        );
    }

    #[test]
    fn schema_rail_and_editor_project_row_types_and_usage() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();

        let rows = schema_rows(&catalog, Some(&schemas), Some("sample::ItemRow"));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "sample::ItemRow");
        assert_eq!(rows[0].used_by_label, "2 tables");
        assert!(rows[0].selected);
        assert!(rows[0].resolved);

        let editor = build_schema_editor(&catalog, Some(&schemas), "sample::ItemRow");
        assert!(editor.resolved);
        assert_eq!(editor.label, "Item Row");
        assert_eq!(
            editor.description.as_deref(),
            Some("Editable item table row.")
        );
        assert_eq!(
            editor
                .fields
                .iter()
                .map(|field| field.key.as_str())
                .collect::<Vec<_>>(),
            vec!["item_id", "display_name", "enabled"]
        );
        assert_eq!(
            editor
                .used_by
                .iter()
                .map(|usage| usage.table_key.as_str())
                .collect::<Vec<_>>(),
            vec!["Items", "ItemsClient"]
        );
    }

    #[test]
    fn manager_groups_split_automatic_providers_from_owner_gems() {
        let mut catalog = catalog_fixture();
        let mut provider = manager_fixture();
        provider.key = "provider:table:Items".to_owned();
        provider.name = "Items provider".to_owned();
        provider.read_only = true;
        catalog.managers.push(provider);

        let groups = manager_groups(&catalog, Some("manager:item-records"));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].kind, GameDataManagerGroupKind::AutomaticProviders);
        assert_eq!(groups[0].managers[0].kind_label, "Auto");
        assert_eq!(groups[1].kind, GameDataManagerGroupKind::OwnerGem);
        assert_eq!(groups[1].name, "sample-gamedata");
        assert!(groups[1].managers[0].selected);
    }

    #[test]
    fn manager_composition_projects_catalog_descriptor_shape() {
        let catalog = catalog_fixture();
        let manager = manager_fixture();

        let composition = build_manager_composition(&catalog, &manager);
        assert_eq!(composition.kind_label, "Authored");
        assert!(composition.status_ok);
        assert_eq!(composition.backing_table_key.as_deref(), Some("Items"));
        assert_eq!(
            composition
                .stages
                .iter()
                .map(|stage| stage.key.as_str())
                .collect::<Vec<_>>(),
            vec!["sources", "filters", "keys", "projection", "output"]
        );
        assert_eq!(composition.stages[0].detail, "2 declared");

        assert_eq!(composition.sources.len(), 2);
        assert_eq!(composition.sources[0].table_key.as_deref(), Some("Items"));
        assert_eq!(composition.sources[1].key, "manager:Localization provider");
        assert!(composition.sources[1].table_key.is_none());

        assert_eq!(composition.projection_rows.len(), 2);
        assert!(!composition.projection_rows[0].computed);
        assert!(composition.projection_rows[1].computed);
        assert_eq!(
            composition.key_policy_summary,
            "CRC32 · trim · lowercase · reject zero CRC"
        );
        assert_eq!(
            composition.transform_chips,
            vec!["identity", "lowercase", "lowercase crc string", "trim"]
        );
        assert!(composition.validation[0].ok);
    }

    #[test]
    fn inspector_tabs_follow_selection_kind_and_field_selection() {
        let catalog = catalog_fixture();
        let schemas = schema_catalog_fixture();

        let projection =
            build_game_data_projection(Some(&catalog), Some(&schemas), &GameDataUiState::default());
        assert!(projection.catalog_loaded);
        assert_eq!(projection.inspector.tabs, vec![GameDataInspectorTab::Table]);
        assert_eq!(projection.inspector.tab, GameDataInspectorTab::Table);

        let state = state_with(|state| {
            state.selected_field_key = Some("item_id".into());
            state.inspector_tab = Some(GameDataInspectorTab::Field);
        });
        let projection = build_game_data_projection(Some(&catalog), Some(&schemas), &state);
        assert_eq!(
            projection.inspector.tabs,
            vec![GameDataInspectorTab::Field, GameDataInspectorTab::Table]
        );
        assert_eq!(projection.inspector.tab, GameDataInspectorTab::Field);

        let state = state_with(|state| {
            state.selected_kind = GameDataSelectionKind::Manager;
            state.rail_view = GameDataRailView::Managers;
        });
        let projection = build_game_data_projection(Some(&catalog), Some(&schemas), &state);
        assert!(matches!(
            projection.workbench,
            GameDataWorkbenchProjection::Manager(_)
        ));
        assert_eq!(projection.inspector.tab, GameDataInspectorTab::Manager);
    }

    #[test]
    fn missing_catalog_projects_unloaded_state() {
        let projection = build_game_data_projection(None, None, &GameDataUiState::default());
        assert!(!projection.catalog_loaded);
        assert!(matches!(
            projection.workbench,
            GameDataWorkbenchProjection::Empty { .. }
        ));
    }
}
