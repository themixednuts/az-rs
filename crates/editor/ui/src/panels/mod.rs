//! Editor panel components
//!
//! Reusable panel UI components for the `AZoth` Editor.

pub mod asset_browser;
pub mod asset_creation;
pub mod connection;
pub mod console;
pub mod gamedata_projection;
pub mod gems;
pub mod graph;
pub mod inspector;
pub mod kit;
pub mod layers;
pub mod materials_projection;
pub mod modes;
pub mod outliner;
pub mod output_log;
pub mod profiler;
pub mod project_workflow;
pub mod scripting_projection;
pub mod session;
pub mod type_registry;
pub mod viewport;

pub use asset_browser::{
    AssetBrowser, AssetBrowserEntryData, AssetBrowserEntryStatus, AssetBrowserFolderData,
    AssetBrowserJobData, AssetBrowserJobStatus, AssetBuilderData, AssetBuilderPatternData,
    AssetBuilderPatternKindData, AssetSourceDependentJobData, AssetSourceDependentSourceData,
    AssetSourceFileTemplateData, AssetSourceFileWorkflowData, AssetSourceSchemaAuthoringData,
    AssetSourceSchemaData, CatalogProductData, EditorAssetBrowserStatus, EditorAssetBuilderCatalog,
    EditorAssetProcessorActivity, EditorAssetSourceDependentsPreview, EditorCatalogProductsStatus,
    EditorJobInspection, JobDependencyData, JobProductData, ViewMode, WorkspaceRootData,
    asset_browser_entry_matches_folder, asset_browser_folder_for_key, asset_browser_folders,
};
pub use asset_creation::{
    AssetSourceCreateRequestData, AssetSourceCreateRequestError, CreatableAssetSourceData,
    build_asset_source_create_request, creatable_asset_sources, default_target_folder_for_source,
    enforce_source_extension, extension_hint, source_path_matches_extensions,
    validate_asset_db_relative_path,
};
pub use connection::{
    EditorProjectConnectionState, EditorProjectConnectionStateData, editor_project_host_connecting,
    editor_project_host_failed_reason, render_project_host_connecting_placeholder,
    render_project_host_connection_placeholder, render_project_host_failed_placeholder,
};
pub use console::{
    Console, ConsoleLevelCounts, ConsoleState, ConsoleView, LogLevel, LogMessage,
    format_console_time,
};
pub use gamedata_projection::{
    EditorGameDataProjection, GameDataDiagnosticProjection, GameDataFieldDetailProjection,
    GameDataFieldKind, GameDataGridCellProjection, GameDataGridColumnProjection,
    GameDataGridProjection, GameDataGridRowProjection, GameDataGridRowState,
    GameDataInspectorProjection, GameDataInspectorTab, GameDataManagerCompositionProjection,
    GameDataManagerFilterRowProjection, GameDataManagerGroupKind, GameDataManagerGroupProjection,
    GameDataManagerNodeChipProjection, GameDataManagerProjectionRowProjection,
    GameDataManagerRowProjection, GameDataManagerSourceProjection, GameDataManagerStageProjection,
    GameDataManagerValidationRowProjection, GameDataRailView, GameDataSchemaEditorProjection,
    GameDataSchemaFieldProjection, GameDataSchemaRowProjection, GameDataSchemaUsageProjection,
    GameDataTableFolderProjection, GameDataTableRowProjection, GameDataTone,
    GameDataWorkbenchProjection,
};
pub use gems::GemsPanel;
pub use graph::{
    EditorGraphDocumentProjection, GraphBuildJobProjectionData, GraphBuildJobStatusData,
    GraphBuildSourceStatusData, GraphBuildStatusProjectionData, GraphCommentProjectionData,
    GraphCompilerBackendKindProjectionData, GraphCompilerBackendProjectionData,
    GraphConnectionProjectionData, GraphCreationCatalogProjectionData,
    GraphDocumentListItemProjectionData, GraphDocumentListProjectionData,
    GraphDocumentProjectionData, GraphGeneratedRustAbiProjectionData,
    GraphInputValueProjectionData, GraphLayoutJobData, GraphLayoutJobPhaseData,
    GraphNodePaletteItemData, GraphNodePaletteProjectionData, GraphNodeProjectionData,
    GraphNodeRuntimeBindingProjectionData, GraphNodeSourceLinkProjectionData,
    GraphPointProjectionData, GraphPortDirectionData, GraphPortProjectionData, GraphPortSideData,
    GraphRouteAnchorKindData, GraphRouteAnchorProjectionData,
    GraphRuntimeExecutionStrategyProjectionData, GraphRustCallResultProjectionData,
    GraphRustNodeCallAbiProjectionData, GraphTypeCreationProjectionData, VisualGraphPanel,
};
pub use inspector::{
    AuthoredInspector, EditorAddableAuthoredComponents, EditorReflectedSelectionState,
};
pub use layers::{
    AuthoredLayerRow, EditorLayerVisibility, SceneLayersPanel, authored_layer_rows,
    authored_object_visibility_key,
};
pub use materials_projection::{
    EditorMaterialsProjection, MaterialCanvasProjection, MaterialDocumentKind,
    MaterialDocumentTabProjection, MaterialGraphTargetProjection,
    MaterialPaletteCategoryProjection, MaterialPaletteItemProjection, MaterialPaletteProjection,
    MaterialParamControlProjection, MaterialParamRowProjection, MaterialParamsProjection,
    MaterialPreviewMaterialProjection, MaterialPreviewProjection, MaterialPreviewShape,
};
pub use modes::{
    AnimationMannequinPanel, AnimationMotionTrackPanel, AnimationNavigatorPanel,
    AnimationWorkbenchPanel, GameDataCatalogPanel, GameDataDetailsPanel, GameDataWorkbenchPanel,
    MaterialPalettePanel, MaterialParamsPanel, MaterialWorkbenchPanel, ScriptEditorPanel,
    ScriptFilesPanel, ScriptOutlinePanel, SequencerWorkbenchPanel,
};
pub use outliner::{
    AuthoredDocumentOutlineData, AuthoredObjectOutlineData, AuthoredOutlineData, AuthoredOutliner,
    ComponentCapabilityData, CreatableAuthoredSchemaData, ENGINE_PREFAB_ENTITY_SCHEMA_TYPE,
    ENGINE_PREFAB_ROOT_SCHEMA_TYPE, ENGINE_SCENE_ROOT_SCHEMA_TYPE, EditorActiveLevel,
    EditorAuthoredOutline, EditorCreatableAuthoredSchemas, active_level_document,
    active_level_prefab_documents, is_scene_document, is_scene_document_schema,
};
pub use output_log::{OutputLogMessage, OutputLogPanel, OutputLogState};
pub use profiler::{
    ProfilerPanel, ProfilerPipelineStatus, gpu_pipeline_status, profiler_backend_label,
    profiler_diagnostic_count, profiler_frame_extent_label, profiler_generation_label,
    runtime_pipeline_status, viewport_pipeline_status,
};
pub use scripting_projection::{
    EditorScriptingProjection, ScriptDocumentTabProjection, ScriptFileRowProjection,
    ScriptFolderProjection, ScriptOutlineRowKind, ScriptOutlineRowProjection,
    ScriptTextFileRowProjection, ScriptWorkbenchProjection, script_text_folders, script_text_key,
    script_text_row_by_key, script_text_source_roots,
};
pub use session::{
    EditorSessionStateData, EditorSessionStatus, SessionPanel, SessionProcessData,
    SessionProcessStateData, SessionServiceRoleData,
};
pub use type_registry::EditorTypeRegistry;
pub use viewport::ViewportPanel;
pub use viewport::{
    DEFAULT_MANNEQUIN_CHARACTER_GLB, DEFAULT_MANNEQUIN_MOTION_GLB, EditorAnimationEventData,
    EditorAnimationJointData, EditorAnimationMotionData, EditorAnimationPreviewCatalog,
    EditorBlendSpaceAssetData, EditorBlendSpaceAssetKind, EditorBlendSpaceCoordinateData,
    EditorBlendSpaceData, EditorBlendSpaceDimensionData, EditorBlendSpaceExampleData,
    EditorBlendSpacePreview, EditorBlendSpacePreviewCatalog, EditorBlendSpaceVirtualExampleData,
    EditorBlendSpaceWeightData, EditorGpuStateData, EditorGpuStatus,
    EditorMannequinAnimationRefData, EditorMannequinAuthoringCatalog,
    EditorMannequinFragmentBlendData, EditorMannequinFragmentData,
    EditorMannequinFragmentDefinitionData, EditorMannequinFragmentOptionData,
    EditorMannequinFragmentOverrideData, EditorMannequinPreview,
    EditorMannequinResolvedAnimationData, EditorMannequinScopeContextData,
    EditorMannequinScopeData, EditorMannequinTagData, EditorMannequinTagGroupData,
    EditorRuntimeProjectionCatalog, EditorRuntimeStateData, EditorRuntimeStatus,
    EditorViewportCameraState, EditorViewportInputQueue, EditorViewportPanelFrame,
    EditorViewportRenderStateData, EditorViewportRenderStatus, EditorViewportTelemetryData,
    InProcessViewportSurface, RuntimeProjectionData, ViewportAssetDrag, ViewportCameraDragKind,
    ViewportCameraView, ViewportCompositionLayout, ViewportDeviceRect, ViewportInputEvent,
    ViewportShadingMode, ViewportSurfaceOwner, ViewportUiInputTrace, ViewportVisibilitySettings,
    set_viewport_input_trace_sink, triad_axis_directions, viewport_device_rect,
};

use gpui::{App, AppContext};
use gpui_component::dock::{PanelView, register_panel};

/// Register all default editor panels into the global `PanelRegistry`.
///
/// Split along the dock the panel belongs to; the registry keys on the panel
/// name, so the three groups only have to run, not run in any order.
pub fn register_default_panels(cx: &mut App) {
    register_workspace_panels(cx);
    register_mode_workbench_panels(cx);
    register_mode_side_panels(cx);
}

/// The always-present workspace shell: outliner, inspector, asset browser,
/// console, and the other panels a session opens with regardless of mode.
fn register_workspace_panels(cx: &mut App) {
    register_panel(
        cx,
        AuthoredOutliner::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| AuthoredOutliner::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        AuthoredInspector::NAME,
        |_dock, state, _info, _window, cx| {
            let state = state.clone();
            let entity = cx.new(move |cx| AuthoredInspector::init_from_panel_state(&state, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        AssetBrowser::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| AssetBrowser::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(cx, Console::NAME, |_dock, _state, _info, window, cx| {
        let entity = cx.new(|cx| Console::init(window, cx));
        Box::new(entity) as Box<dyn PanelView>
    });
    register_panel(
        cx,
        SceneLayersPanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| SceneLayersPanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        OutputLogPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(OutputLogPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        ProfilerPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(ProfilerPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(cx, GemsPanel::NAME, |_dock, _state, _info, window, cx| {
        let entity = cx.new(|cx| GemsPanel::init(window, cx));
        Box::new(entity) as Box<dyn PanelView>
    });
    register_panel(
        cx,
        SessionPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(SessionPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        ViewportPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(ViewportPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        VisualGraphPanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| VisualGraphPanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
}

/// The center workbench each authoring mode docks: material graph, script
/// editor, data table, sequencer, animation.
fn register_mode_workbench_panels(cx: &mut App) {
    register_panel(
        cx,
        MaterialWorkbenchPanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| MaterialWorkbenchPanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        ScriptEditorPanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| ScriptEditorPanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        GameDataWorkbenchPanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| GameDataWorkbenchPanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        SequencerWorkbenchPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(SequencerWorkbenchPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        AnimationWorkbenchPanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| AnimationWorkbenchPanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        AnimationMotionTrackPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(AnimationMotionTrackPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        AnimationMannequinPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(AnimationMannequinPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
}

/// The navigator and detail panels the authoring modes dock beside their
/// workbench.
fn register_mode_side_panels(cx: &mut App) {
    register_panel(
        cx,
        MaterialPalettePanel::NAME,
        |_dock, _state, _info, window, cx| {
            let entity = cx.new(|cx| MaterialPalettePanel::init(window, cx));
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        MaterialParamsPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(MaterialParamsPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        ScriptFilesPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(ScriptFilesPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        ScriptOutlinePanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(ScriptOutlinePanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        GameDataCatalogPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(GameDataCatalogPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        GameDataDetailsPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(GameDataDetailsPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
    register_panel(
        cx,
        AnimationNavigatorPanel::NAME,
        |_dock, _state, _info, _window, cx| {
            let entity = cx.new(AnimationNavigatorPanel::init);
            Box::new(entity) as Box<dyn PanelView>
        },
    );
}
