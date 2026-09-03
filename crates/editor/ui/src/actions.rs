//! Editor actions
//!
//! Defines actions that can be triggered from menus, keyboard shortcuts, etc.

// gpui's `actions!` macro expands to action structs that derive `PartialEq`
// but not `Eq`; the expansion is outside our control, so the lint is suppressed
// for this generated-code-only module.
#![allow(
    clippy::derive_partial_eq_without_eq,
    reason = "gpui `actions!` macro derives PartialEq without Eq; expansion is not ours to change"
)]

use az_core::reflect::ReflectedValueEnvelope;
use az_editor_inspector::ReflectedEditBinding;
use az_proto_project::vnext::PrefabEditCommand;
use gpui::actions;

use crate::panels::gamedata_projection::{GameDataInspectorTab, GameDataRailView};
use crate::panels::materials_projection::MaterialPreviewShape;
use crate::scene_tools::{EditorScenePivot, EditorSceneToolKind, EditorSceneTransformSpace};

actions!(
    editor,
    [
        /// Create a new project
        NewProject,
        /// Open an existing project
        OpenProject,
        /// Save the current project
        Save,
        /// Save the current project with a new name
        SaveAs,
        /// Quit the application
        Quit,
        /// Undo the last action
        Undo,
        /// Redo the last undone action
        Redo,
        /// Cut selected content
        Cut,
        /// Copy selected content
        Copy,
        /// Paste content
        Paste,
        /// Toggle the authored outliner panel
        ToggleOutliner,
        /// Toggle the inspector panel
        ToggleInspector,
        /// Toggle the asset browser panel
        ToggleAssetBrowser,
        /// Pump latency-sensitive viewport input immediately.
        PumpViewportInput,
        /// Frame the selected authored entity using its resolved mesh bounds.
        FrameSelected,
        /// Open or focus the live Asset Processor window
        OpenAssetProcessor,
        /// Toggle the session panel
        ToggleSessionPanel,
        /// Toggle the visual graph panel
        ToggleGraphPanel,
        /// Reconcile watched source roots through the session asset processor
        ScanAssets,
        /// Refresh current product assets from the session asset processor
        RefreshAssetProducts,
        /// Refresh asset browser status from the session asset processor
        RefreshAssets,
        /// Load the next asset browser status page from the session asset processor
        LoadMoreAssets,
        /// Refresh session supervision status from session-supervisor
        RefreshSessionStatus,
        /// Start planned or cleanly exited services owned by the attached session
        StartSessionServices,
        /// Stop all services owned by the attached session
        StopSessionServices,
        /// Recover a failed-preserved session
        RecoverSession,
        /// Force recovery of a failed-preserved session
        ForceRecoverSession,
        /// Launch the editor-world runtime projection
        LaunchEditorWorld,
        /// Refresh editor-world runtime status metadata
        RefreshRuntimeStatus,
        /// Plan the selected project build through the user daemon
        PlanProjectBuild,
        /// Execute the selected project build through the user daemon
        ExecuteProjectBuild,
        /// Toggle editor grid snap state
        ToggleSceneGridSnap,
        /// Toggle editor angle snap state
        ToggleSceneAngleSnap,
        /// Refresh the latest editor-world viewport frame metadata
        RefreshViewportFrame,
        /// Refresh runtime projection inventory from runtime-host
        RefreshRuntimeProjections,
        /// Refresh the selected graph document from project-host
        RefreshGraphDocument,
        /// Run graph auto layout through the editor graph adapter
        AutoLayoutGraph,
        /// Reroute graph connections through the editor graph adapter
        RouteGraphConnections,
        /// Save the selected visual graph document
        SaveGraphDocument,
        /// Save and enqueue runtime products for the selected visual graph document
        BuildGraphDocument,
        /// Refresh the selected visual graph document's asset build status
        RefreshGraphBuildStatus,
        /// Toggle the console panel
        ToggleConsole,
        /// Toggle fullscreen mode
        ToggleFullscreen,
        /// Reset the current mode's dock layout to its design defaults
        ResetLayout,
        /// Minimize the window
        Minimize,
        /// Zoom the window
        Zoom,
        /// Show editor preferences
        Preferences,
        /// Show about dialog
        About,
        /// Dismiss the top-most editor overlay
        DismissOverlay,
        /// Return to the project launcher from a connecting/failed project open
        BackToProjectLauncher,
    ]
);

/// The closed set of session-attached editor controllers that can be retried.
///
/// The UI owns this action payload so a failed controller can be retried from
/// a central status surface without making `az-editor-ui` depend on core's
/// lifecycle aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachedControllerKind {
    ReflectedSelection,
    GameData,
    AssetBrowser,
    Graph,
    MannequinAnimation,
    Sequencer,
    Recovery,
    SessionStatus,
    ProjectBuild,
    Runtime,
}

/// Retry one explicitly failed session-attached controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RetryAttachedController {
    pub controller: AttachedControllerKind,
}

/// Applies a command built from a reflected inspector edit binding.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct ApplyReflectedPrefabEdit {
    pub command: PrefabEditCommand,
}

/// Invokes an editor action attached to a reflected component or field.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct InvokeReflectedInspectorAction {
    pub binding: ReflectedEditBinding,
    pub action_id: String,
}

/// Traverses project-host-owned reflected Prefab history backward.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct UndoReflectedPrefabEdit;

/// Traverses project-host-owned reflected Prefab history forward.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RedoReflectedPrefabEdit;

/// Reloads the selected reflected Prefab entity from project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RefreshReflectedInspection;

/// Add a schema-typed component object to the selected prefab entity.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct AddAuthoredComponent {
    pub schema_type: String,
}

/// Select an authored object through the editor-owned project-host controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectAuthoredObject {
    pub document_id: String,
    pub object_id: String,
}

/// Select an authored document through the editor-owned project-host controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectAuthoredDocument {
    pub document_id: String,
}

/// Create a new authored document through the editor-owned project-host controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CreateAuthoredDocument {
    pub root_schema: String,
}

/// Refresh `GameData` catalog descriptors from project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RefreshGameDataCatalog;

/// Switch the `GameData` catalog rail between tables, schemas, and managers.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetGameDataRailView {
    pub view: GameDataRailView,
}

/// Select a `GameData` table by catalog key through the editor-owned `GameData`
/// catalog controller. Loads the backing authored document for the grid.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectGameDataTable {
    pub table_key: String,
}

/// Select a `GameData` row schema by schema key.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectGameDataSchema {
    pub schema_key: String,
}

/// Select a `GameData` manager by catalog key.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectGameDataManager {
    pub manager_key: String,
}

/// Select a field of the current `GameData` table for the field inspector.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectGameDataField {
    pub field_key: String,
}

/// Switch the `GameData` details inspector tab.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetGameDataInspectorTab {
    pub tab: GameDataInspectorTab,
}

/// Set the `GameData` grid row filter text.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetGameDataGridFilter {
    pub filter: String,
}

/// Add a schema-derived default row object to a `GameData` table document.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct AddGameDataRow {}

/// Duplicate one row object in a `GameData` table document.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct DuplicateGameDataRow {
    pub object_id: String,
}

/// Remove one row object from a `GameData` table document.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RemoveGameDataRow {
    pub object_id: String,
}

/// Undo through the active `GameData` generic source session only.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct UndoGameDataRows;

/// Redo through the active `GameData` generic source session only.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RedoGameDataRows;

/// Close the active `GameData` generic source session.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CloseGameDataTable;

/// Refresh the authored document/object outline from project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RefreshAuthoredOutline;

/// Set an authored document layer's editor visibility.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetLayerVisibility {
    pub document_id: String,
    pub visible: bool,
}

/// Set an authored object's editor-only viewport visibility.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetAuthoredObjectVisibility {
    pub document_id: String,
    pub object_id: String,
    pub visible: bool,
}

/// Set an authored document layer's editor lock state.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetLayerLock {
    pub document_id: String,
    pub locked: bool,
}

/// Execute a command through the editor-owned console controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct ExecuteConsoleCommand {
    pub command: String,
}

/// Inspect a job attempt through the editor-owned asset processor controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct InspectJob {
    pub job_id: i64,
    pub attempt_id: Option<i64>,
}

/// Create a file-backed asset source through the editor-owned asset processor controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CreateAssetSourceFile {
    pub schema_type: String,
    pub source_root: String,
    pub source_path: String,
}

/// Query dependents before confirming deletion of a file-backed asset source.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct PreviewDeleteAssetSource {
    pub source_root: String,
    pub source_path: String,
}

/// Delete a file-backed asset source through the editor-owned asset processor controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct DeleteAssetSource {
    pub source_root: String,
    pub source_path: String,
}

/// Rename or move a file-backed asset source through the editor-owned asset processor controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RenameAssetSource {
    pub source_root: String,
    pub from_source_path: String,
    pub to_source_path: String,
}

/// Force the asset processor to enqueue new jobs for an existing source.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct ForceReprocessAsset {
    pub source_root: String,
    pub source_path: String,
}

/// Refresh catalog products for a concrete platform through the editor-owned asset processor controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RefreshCatalogProducts {
    pub platform: String,
}

/// Select a material document (graph, material instance, or material type)
/// through the editor-owned materials controller.
///
/// Graph sources load into the visual-graph controller; property documents
/// load through the authored-selection controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectMaterialDocument {
    pub document_id: String,
}

/// Switch the material preview swatch geometry.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetMaterialPreviewShape {
    pub shape: MaterialPreviewShape,
}

/// Set the material node palette filter text.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetMaterialPaletteFilter {
    pub filter: String,
}

/// Select a script tree row through the editor-owned scripting controller.
///
/// Graph-canvas rows load into the visual-graph controller; text-asset rows
/// are recorded for the workbench file card (no content-load RPC exists yet
/// — see [`OpenScriptInExternalEditor`]).
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectScriptFile {
    pub key: String,
}

/// Open a text script source in the OS default (or configured)
/// external editor, resolved from its source root + relative path.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct OpenScriptInExternalEditor {
    pub source_root: String,
    pub source_path: String,
}

/// Create a visual graph document through the editor-owned project-host controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CreateGraphDocument {
    pub graph_type: String,
    pub document_name: String,
}

/// Open an existing visual graph document through the editor-owned project-host controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct OpenGraphDocument {
    pub document_id: String,
}

/// Open or inspect a source target published by a visual graph node descriptor.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct OpenGraphNodeSourceLink {
    pub package: Option<String>,
    pub module_path: Option<String>,
    pub symbol_path: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub docs_url: Option<String>,
}

/// Add a node to the selected visual graph document through project-host.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct AddGraphNode {
    pub node_type: String,
    pub node_type_version: u32,
    pub x: f32,
    pub y: f32,
}

/// Move a node in the selected visual graph document through project-host.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct MoveGraphNode {
    pub node_id: String,
    pub x: f32,
    pub y: f32,
}

/// Remove a node from the selected visual graph document through project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RemoveGraphNode {
    pub node_id: String,
}

/// Move a user waypoint on a graph connection route through project-host.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct MoveGraphRouteAnchor {
    pub connection_id: String,
    pub anchor_id: String,
    pub x: f32,
    pub y: f32,
}

/// Create a comment/note in the selected visual graph document through project-host.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CreateGraphComment {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Move or resize a graph comment through project-host.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct MoveGraphComment {
    pub comment_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// Update graph comment text through project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetGraphCommentText {
    pub comment_id: String,
    pub text: String,
}

/// Remove a graph comment through project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RemoveGraphComment {
    pub comment_id: String,
}

/// Connect two ports in the selected visual graph document through project-host.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct ConnectGraphPorts {
    pub from_node_id: String,
    pub from_port_id: u32,
    pub to_node_id: String,
    pub to_port_id: u32,
}

/// Sets or clears a reflected value on an input port in the selected graph.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetReflectedGraphPortValue {
    pub node_id: String,
    pub port_id: u32,
    pub value: Option<ReflectedValueEnvelope>,
}

/// Create a new Azoth project through the editor-owned project workflow controller.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CreateProject {
    pub name: String,
    pub path: String,
    pub lore_url: Option<String>,
    pub topology: String,
    pub enabled_gems: Vec<String>,
}

/// Create a new project gem through the editor-owned project workflow.
///
/// Mirrors `azoth gem new` (including the ADR 0026
/// `--capability provider|session-authority` templates). The editor shell
/// resolves the target project from the attached session.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct CreateGem {
    /// Gem name (validated like a project name by the scaffold).
    pub name: String,
    /// Optional explicit gem id; the scaffold derives one from the name when empty.
    pub id: Option<String>,
    /// Optional capability template id (`provider` / `session-authority`).
    pub capability: Option<String>,
    /// Register the gem with the attached project (on by default).
    pub register: bool,
}

/// Initialize Azoth project workflow files in an existing project directory.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct InitializeProjectWorkflow {
    pub path: String,
    pub name: Option<String>,
    pub lore_url: Option<String>,
}

/// Ensure a project supervision session exists through the user daemon.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct EnsureProjectWorkflowSession {
    pub project_root: String,
    pub session_name: String,
}

/// Prepare project session services through the user daemon.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct PrepareProjectWorkflowSessionServices {
    pub project_root: String,
    pub session_slug: String,
}

/// Ensure project session services are running and attach this editor instance.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct OpenProjectWorkflowEditorSession {
    pub project_root: String,
    pub session_name: String,
}

/// Stop the editor-world runtime projection.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct StopEditorWorld {
    pub preserve: bool,
}

/// Select the mannequin animation preview character asset.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectAnimationCharacter {
    pub character_glb: String,
}

/// Select the mannequin animation preview motion asset.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectAnimationMotion {
    pub motion_glb: String,
}

/// Select the mannequin animation blend-space authoring source.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectAnimationBlendSpace {
    pub bspace_ron_path: String,
}

/// Set one blend-space preview parameter.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetAnimationBlendSpaceParameter {
    pub dimension: String,
    pub value: f32,
}

/// Set all blend-space preview parameters at once.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetAnimationBlendSpaceParameters {
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnimationBlendSpaceCoordinateEdit {
    pub dimension: String,
    pub value: f32,
}

/// Edit one authored blend-space example coordinate.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct EditAnimationBlendSpaceExampleCoordinate {
    pub example_index: u32,
    pub dimension: String,
    pub value: f32,
}

/// Add an authored blend-space example.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct AddAnimationBlendSpaceExample {
    pub animation_name: String,
    pub motion_path: String,
    pub coordinates: Vec<AnimationBlendSpaceCoordinateEdit>,
}

/// Remove an authored blend-space example.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RemoveAnimationBlendSpaceExample {
    pub example_index: u32,
}

/// Edit an authored blend-space dimension range.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct EditAnimationBlendSpaceDimensionRange {
    pub dimension: String,
    pub min: f32,
    pub max: f32,
}

/// Select the active Mannequin fragment ID for tag-conditioned preview.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SelectMannequinFragment {
    pub fragment_key: String,
}

/// Set a Mannequin tag in the editor tag state.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetMannequinTag {
    pub tag: String,
    pub enabled: bool,
}

/// Edit one authored Mannequin fragment-option animation reference.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct EditMannequinFragmentOptionAnimation {
    pub fragment_key: String,
    pub option_index: u32,
    pub layer_index: u32,
    pub animation_index: u32,
    pub animation_ref: String,
}

/// Edit one authored Mannequin fragment-option tag condition.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct EditMannequinFragmentOptionTags {
    pub fragment_key: String,
    pub option_index: u32,
    pub tag_condition: String,
}

/// Add an authored Mannequin fragment option.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct AddMannequinFragmentOption {
    pub fragment_key: String,
    pub animation_ref: String,
    pub tag_condition: String,
}

/// Remove an authored Mannequin fragment option.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct RemoveMannequinFragmentOption {
    pub fragment_key: String,
    pub option_index: u32,
}

/// Play or pause the mannequin animation preview.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetAnimationPreviewPlaying {
    pub playing: bool,
}

/// Stop the mannequin animation preview and rewind to the start.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct StopAnimationPreview;

/// Toggle looping on the mannequin animation preview.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetAnimationPreviewLoop {
    pub looping: bool,
}

/// Scrub the mannequin animation preview timeline.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct ScrubAnimationPreview {
    pub position_millis: u32,
}

/// Select the editor scene manipulation tool.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetSceneTool {
    pub tool: EditorSceneToolKind,
}

/// Select the editor scene pivot origin mode.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetScenePivot {
    pub pivot: EditorScenePivot,
}

/// Select the editor scene transform coordinate space.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetSceneTransformSpace {
    pub space: EditorSceneTransformSpace,
}

/// Select the active project build profile.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetProjectBuildProfile {
    pub profile: String,
}

/// Select the active project build cargo target.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SetProjectBuildTarget {
    pub target_key: String,
}

/// Tail a session-owned service log through session-supervisor.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct TailSessionServiceLog {
    pub service_name: String,
    pub run: Option<uuid::Uuid>,
    pub stream: SessionServiceLogStream,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionServiceLogStream {
    Stdout,
    Stderr,
    Structured,
}

/// Register all editor actions with the application context
pub fn register_actions(cx: &mut gpui::App) {
    // Actions are automatically registered via the actions! macro
    // This function can be used for any additional action setup if needed
    cx.on_action(|_: &Quit, cx| {
        // Quit the application
        // This will close all windows and exit the application
        cx.quit();
    });

    cx.on_action(|_: &Minimize, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| {
                    window.minimize_window();
                })
                .ok();
        }
    });

    cx.on_action(|_: &Zoom, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| {
                    window.zoom_window();
                })
                .ok();
        }
    });

    cx.on_action(|_: &ToggleFullscreen, cx| {
        if let Some(window) = cx.active_window() {
            window
                .update(cx, |_, window, _| {
                    window.toggle_fullscreen();
                })
                .ok();
        }
    });
}

/// Play or pause the Sequencer transport.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SeqPlay {
    pub playing: bool,
}

/// Stop the Sequencer transport and rewind to its start.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SeqStop;

/// Scrub the Sequencer transport to an absolute timeline position.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SeqScrub {
    pub position_millis: u32,
}

/// Set whether the Sequencer transport loops at the timeline end.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SeqSetLoop {
    pub looping: bool,
}

/// Select a transport boundary or adjacent read-only event key.
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = editor, no_json)]
pub struct SeqGoToKey {
    pub target: SeqKeyTarget,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeqKeyTarget {
    Start,
    Previous,
    Next,
    End,
}
