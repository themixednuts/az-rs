//! Editor-owned Scripting mode controller.
//!
//! Scripting mode is a projection over two existing controller boundaries,
//! same pattern as Materials ([`crate::materials_ui`]): script *canvases*
//! (visual graph documents) live in the visual-graph controller
//! ([`crate::graph_ui`]) and script *assets* (text sources) live in the
//! asset-browser workspace view
//! ([`az_editor_ui::panels::EditorAssetBrowserStatus`]).
//!
//! Two current capability gaps shape this module:
//!
//! - No dedicated script graph type is registered anywhere yet (only
//!   `az-material-nodes` registers a graph type in production code). Script
//!   canvases are therefore classified by elimination: any visual-graph
//!   document that is not a material graph is a script canvas.
//! - `az_project::ProjectPaths::scripts` (`azoth.toml`) is declared in the
//!   manifest schema but is not wired into the watched asset-pipeline source
//!   roots (only `ProjectPaths::assets` is — see
//!   `az_project::project_source_roots_from_resolved`). Text script assets
//!   are scoped to source roots the asset pipeline has actually registered
//!   as script roots
//!   ([`az_editor_ui::panels::script_text_source_roots`]), which is
//!   legitimately empty until that backend gap closes. This module never
//!   falls back to scanning the whole assets tree for `.lua` files.

use az_editor_ui::panels::{
    EditorAssetBrowserStatus, EditorGraphDocumentProjection, EditorScriptingProjection,
    GraphDocumentListItemProjectionData, GraphDocumentProjectionData, GraphPortDirectionData,
    GraphPortProjectionData, ScriptDocumentTabProjection, ScriptFileRowProjection,
    ScriptOutlineRowKind, ScriptOutlineRowProjection, ScriptTextFileRowProjection,
    ScriptWorkbenchProjection, script_text_row_by_key,
};
use gpui::{App, Global};
use tracing::{error, info};

use crate::mode_projection::{
    ModeProjectionInputs, ModeProjectionRegistration, ModeProjectionRegistrationError,
    ModeProjectionSpec, publish_mode_projection_and_refresh,
};
use crate::settings::SettingsStore;
use crate::source_navigation::{
    default_source_navigation_settings, script_source_navigation_intent,
};

/// Material graph type id — excluded from the script domain (Materials owns
/// it; see `crate::materials_ui::MATERIAL_GRAPH_TYPE`).
const MATERIAL_GRAPH_TYPE: &str = az_material::MATERIAL_GRAPH_ASSET_TYPE_HINT;

/// Editor-core-owned Scripting mode UI state. Input to the projection build;
/// never rendered directly.
#[derive(Debug, Clone, Default)]
struct ScriptingUiState {
    selected_key: String,
}

impl Global for ScriptingUiState {}

struct ScriptingMode;

impl ModeProjectionSpec for ScriptingMode {
    type State = ScriptingUiState;
    type Projection = EditorScriptingProjection;

    const NAME: &'static str = "scripting";

    fn register_inputs(inputs: &mut ModeProjectionInputs) {
        inputs.depends_on::<EditorGraphDocumentProjection>();
        inputs.depends_on::<EditorAssetBrowserStatus>();
    }

    fn install_actions(cx: &mut App) {
        install_scripting_actions(cx);
    }

    fn project(state: &Self::State, cx: &App) -> Self::Projection {
        build_scripting_projection(
            cx.try_global::<EditorGraphDocumentProjection>(),
            cx.try_global::<EditorAssetBrowserStatus>(),
            state,
        )
    }
}

pub fn mode_projection_registration()
-> Result<ModeProjectionRegistration, ModeProjectionRegistrationError> {
    ModeProjectionRegistration::for_spec::<ScriptingMode>()
}

fn install_scripting_actions(cx: &mut App) {
    cx.on_action(|action: &az_editor_ui::actions::SelectScriptFile, cx| {
        select_script_file(cx, action.key.clone());
    });

    cx.on_action(
        |action: &az_editor_ui::actions::OpenScriptInExternalEditor, cx| {
            if let Err(err) =
                open_script_in_external_editor(cx, &action.source_root, &action.source_path)
            {
                error!(error = %err, "failed to open script in external editor");
            }
        },
    );

    info!("installed scripting action handlers");
}

/// Select a script tree row. Graph-canvas keys (a graph document's
/// `document_id`) load into the visual-graph controller; text-asset keys
/// (`text:<entry_id>`, see
/// [`az_editor_ui::panels::script_text_key`]) are just recorded — there is no
/// content-load RPC for script sources yet.
pub fn select_script_file(cx: &mut App, key: String) {
    if let Some(document_id) = script_graph_document_id_for_key(cx, &key)
        && let Err(err) = crate::graph_ui::open_graph_document(cx, document_id.clone())
    {
        error!(
            error = %err,
            document = %document_id,
            "failed to open script graph document"
        );
    }
    cx.default_global::<ScriptingUiState>().selected_key = key;
    publish_mode_projection_and_refresh::<ScriptingMode>(cx);
}

/// Track a script-canvas selection whose graph-document load is already in
/// flight (asset-browser `SelectAuthoredDocument` routing): records the
/// selection and opens the graph document.
pub fn sync_script_selection_from_authored(cx: &mut App, document_id: String) {
    if let Err(err) = crate::graph_ui::open_graph_document(cx, document_id.clone()) {
        error!(
            error = %err,
            document = %document_id,
            "failed to open script graph document"
        );
    }
    cx.default_global::<ScriptingUiState>().selected_key = document_id;
    publish_mode_projection_and_refresh::<ScriptingMode>(cx);
}

fn script_graph_document_id_for_key(cx: &App, key: &str) -> Option<String> {
    cx.try_global::<EditorGraphDocumentProjection>()
        .and_then(|graph| {
            graph
                .graph_documents
                .documents
                .iter()
                .find(|entry| {
                    (entry.document_id == key || entry.source_path == key)
                        && is_script_graph_type(&entry.graph_type)
                })
                .map(|entry| entry.document_id.clone())
        })
}

/// Open a text script source in the OS default (or configured) external
/// editor, resolved from its asset-browser source root + relative path.
fn open_script_in_external_editor(
    cx: &App,
    source_root: &str,
    source_path: &str,
) -> crate::error::EditorResult<()> {
    let settings = cx
        .try_global::<SettingsStore>()
        .map_or_else(default_source_navigation_settings, |store| {
            store.current.source_navigation.clone()
        });
    let intent = script_source_navigation_intent(&settings, source_root, source_path)?;
    cx.open_url(&intent.url);
    info!(
        target = %intent.label,
        url = %intent.url,
        "opened script source in external editor"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Projection build (pure)
// ---------------------------------------------------------------------------

fn is_script_graph_type(graph_type: &str) -> bool {
    graph_type != MATERIAL_GRAPH_TYPE
}

fn build_scripting_projection(
    graph: Option<&EditorGraphDocumentProjection>,
    status: Option<&EditorAssetBrowserStatus>,
    state: &ScriptingUiState,
) -> EditorScriptingProjection {
    let selected = effective_selected(state, graph);
    let graph_scripts = script_graph_rows(graph, selected.as_deref());
    let tabs = script_tabs(graph, selected.as_deref());
    let text_row = selected
        .as_deref()
        .zip(status)
        .and_then(|(key, status)| script_text_row_by_key(status, key));
    let workbench = script_workbench(selected.as_deref(), &graph_scripts, text_row.as_ref());
    let outline = script_outline(&workbench, graph);
    let outline_empty_reason = script_outline_empty_reason(&workbench, &outline);
    let breadcrumb = script_breadcrumb(&workbench, &graph_scripts);

    EditorScriptingProjection {
        graph_scripts,
        tabs,
        selected_key: selected,
        breadcrumb,
        outline,
        outline_empty_reason,
        workbench,
    }
}

fn effective_selected(
    state: &ScriptingUiState,
    graph: Option<&EditorGraphDocumentProjection>,
) -> Option<String> {
    if !state.selected_key.trim().is_empty() {
        return Some(state.selected_key.clone());
    }
    graph
        .and_then(|graph| graph.document.as_ref())
        .filter(|document| is_script_graph_type(&document.graph_type))
        .map(|document| document.document_id.clone())
}

// --- graph rows / tabs ---

fn script_graph_rows(
    graph: Option<&EditorGraphDocumentProjection>,
    selected: Option<&str>,
) -> Vec<ScriptFileRowProjection> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    let mut rows = graph
        .graph_documents
        .documents
        .iter()
        .filter(|entry| is_script_graph_type(&entry.graph_type))
        .map(|entry| script_graph_row(entry, selected))
        .collect::<Vec<_>>();

    if rows.is_empty()
        && let Some(document) = graph.document.as_ref()
        && is_script_graph_type(&document.graph_type)
    {
        rows.push(script_graph_row_from_document(document, selected));
    }
    rows
}

fn script_graph_row(
    entry: &GraphDocumentListItemProjectionData,
    selected: Option<&str>,
) -> ScriptFileRowProjection {
    ScriptFileRowProjection {
        selected: selected.is_some_and(|key| key == entry.document_id || key == entry.source_path),
        name: source_stem(non_empty_str(&entry.source_path, &entry.document_id)),
        source_path: entry.source_path.clone(),
        unsaved_changes: entry.unsaved_changes,
        has_diagnostics: false,
        key: entry.document_id.clone(),
    }
}

fn script_graph_row_from_document(
    document: &GraphDocumentProjectionData,
    selected: Option<&str>,
) -> ScriptFileRowProjection {
    ScriptFileRowProjection {
        selected: selected.is_some_and(|key| key == document.document_id),
        name: non_empty_str(&document.document_id, &document.graph_type).to_owned(),
        source_path: document.document_id.clone(),
        unsaved_changes: document.unsaved_changes,
        has_diagnostics: !document.diagnostics.is_empty(),
        key: document.document_id.clone(),
    }
}

fn script_tabs(
    graph: Option<&EditorGraphDocumentProjection>,
    selected: Option<&str>,
) -> Vec<ScriptDocumentTabProjection> {
    let Some(graph) = graph else {
        return Vec::new();
    };
    graph
        .graph_documents
        .documents
        .iter()
        .filter(|entry| entry.loaded && is_script_graph_type(&entry.graph_type))
        .map(|entry| ScriptDocumentTabProjection {
            key: entry.document_id.clone(),
            label: source_stem(non_empty_str(&entry.source_path, &entry.document_id)),
            selected: selected.is_some_and(|key| key == entry.document_id),
            unsaved_changes: entry.unsaved_changes,
        })
        .collect()
}

// --- workbench / breadcrumb / outline ---

fn script_workbench(
    selected: Option<&str>,
    graph_scripts: &[ScriptFileRowProjection],
    text_row: Option<&ScriptTextFileRowProjection>,
) -> ScriptWorkbenchProjection {
    let Some(selected) = selected else {
        return ScriptWorkbenchProjection::Empty {
            message: "Select a script from the left file tree.".to_owned(),
        };
    };
    if let Some(row) = graph_scripts.iter().find(|row| row.key == selected) {
        return ScriptWorkbenchProjection::Graph {
            document_id: row.key.clone(),
            unsaved_changes: row.unsaved_changes,
        };
    }
    if let Some(row) = text_row {
        return ScriptWorkbenchProjection::Text(row.clone());
    }
    ScriptWorkbenchProjection::Empty {
        message: "Selected script is no longer available.".to_owned(),
    }
}

fn script_breadcrumb(
    workbench: &ScriptWorkbenchProjection,
    graph_scripts: &[ScriptFileRowProjection],
) -> Vec<String> {
    match workbench {
        ScriptWorkbenchProjection::Graph { document_id, .. } => {
            let path = graph_scripts
                .iter()
                .find(|row| &row.key == document_id)
                .map_or_else(|| document_id.clone(), |row| row.source_path.clone());
            path_segments(&path)
        }
        ScriptWorkbenchProjection::Text(row) => path_segments(&row.source_path),
        ScriptWorkbenchProjection::Empty { .. } => Vec::new(),
    }
}

fn path_segments(path: &str) -> Vec<String> {
    path.replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

fn script_outline(
    workbench: &ScriptWorkbenchProjection,
    graph: Option<&EditorGraphDocumentProjection>,
) -> Vec<ScriptOutlineRowProjection> {
    let ScriptWorkbenchProjection::Graph { document_id, .. } = workbench else {
        return Vec::new();
    };
    let Some(document) = graph.and_then(|graph| graph.document.as_ref()) else {
        return Vec::new();
    };
    if &document.document_id != document_id {
        return Vec::new();
    }

    let mut rows = Vec::new();
    for node in &document.nodes {
        rows.push(ScriptOutlineRowProjection {
            key: node.node_id.clone(),
            label: non_empty_str(&node.label, &node.node_type).to_owned(),
            depth: 0,
            kind: ScriptOutlineRowKind::Node,
        });
        rows.extend(
            node.ports
                .iter()
                .map(|port| script_outline_port_row(node, port)),
        );
    }
    rows.extend(
        document
            .diagnostics
            .iter()
            .enumerate()
            .map(|(index, diagnostic)| ScriptOutlineRowProjection {
                key: format!("diagnostic:{index}"),
                label: diagnostic.clone(),
                depth: 0,
                kind: ScriptOutlineRowKind::Diagnostic,
            }),
    );
    rows
}

fn script_outline_port_row(
    node: &az_editor_ui::panels::GraphNodeProjectionData,
    port: &GraphPortProjectionData,
) -> ScriptOutlineRowProjection {
    let input = matches!(port.direction, GraphPortDirectionData::Input);
    ScriptOutlineRowProjection {
        key: format!("{}:{}", node.node_id, port.port_id),
        label: non_empty_str(&port.name, if input { "input" } else { "output" }).to_owned(),
        depth: 1,
        kind: ScriptOutlineRowKind::Port { input },
    }
}

fn script_outline_empty_reason(
    workbench: &ScriptWorkbenchProjection,
    outline: &[ScriptOutlineRowProjection],
) -> String {
    if !outline.is_empty() {
        return String::new();
    }
    match workbench {
        ScriptWorkbenchProjection::Empty { .. } => "Select a script to see its outline.".to_owned(),
        ScriptWorkbenchProjection::Graph { .. } => {
            "This script canvas has no nodes yet.".to_owned()
        }
        ScriptWorkbenchProjection::Text(_) => "No symbol source for text scripts yet — outline \
             requires a language server or parser integration."
            .to_owned(),
    }
}

// --- helpers ---

fn non_empty_str<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if preferred.trim().is_empty() {
        fallback
    } else {
        preferred
    }
}

/// "`scripts/ai/open_chest.azgraph.ron`" -> "`open_chest`".
fn source_stem(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    let file_name = normalized.rsplit('/').next().unwrap_or(path);
    file_name
        .split_once('.')
        .map_or(file_name, |(stem, _)| stem)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::panels::{
        AssetBrowserEntryData, AssetBrowserEntryStatus, GraphDocumentListItemProjectionData,
        GraphDocumentListProjectionData, GraphNodePaletteProjectionData, GraphNodeProjectionData,
        WorkspaceRootData,
    };

    fn graph_list_item(
        document_id: &str,
        graph_type: &str,
        source_path: &str,
        loaded: bool,
        current: bool,
        unsaved_changes: bool,
    ) -> GraphDocumentListItemProjectionData {
        GraphDocumentListItemProjectionData {
            document_id: document_id.to_owned(),
            graph_type: graph_type.to_owned(),
            source_path: source_path.to_owned(),
            revision: 1,
            saved_revision: Some(1),
            unsaved_changes,
            loaded,
            current,
        }
    }

    fn graph_document(document_id: &str, graph_type: &str) -> GraphDocumentProjectionData {
        GraphDocumentProjectionData {
            document_id: document_id.to_owned(),
            graph_type: graph_type.to_owned(),
            graph_type_info: None,
            revision: 1,
            saved_revision: Some(1),
            unsaved_changes: false,
            catalog_version: 1,
            nodes: vec![GraphNodeProjectionData {
                node_id: "node-1".to_owned(),
                node_type: "azoth.script.print".to_owned(),
                label: "Print".to_owned(),
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 60.0,
                selected: false,
                source_links: Vec::new(),
                ports: vec![GraphPortProjectionData {
                    port_id: 1,
                    name: "message".to_owned(),
                    direction: GraphPortDirectionData::Input,
                    side: az_editor_ui::panels::GraphPortSideData::West,
                    value: None,
                    x: 0.0,
                    y: 0.0,
                }],
            }],
            connections: Vec::new(),
            comments: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn graph_projection_fixture() -> EditorGraphDocumentProjection {
        EditorGraphDocumentProjection::document(graph_document(
            "scripts/ai.azgraph.ron",
            "azoth.script.graph",
        ))
        .with_graph_documents(GraphDocumentListProjectionData::new(vec![
            graph_list_item(
                "scripts/ai.azgraph.ron",
                "azoth.script.graph",
                "scripts/ai.azgraph.ron",
                true,
                true,
                false,
            ),
            graph_list_item(
                "materials/graphs/metal.azmat.ron",
                MATERIAL_GRAPH_TYPE,
                "materials/graphs/metal.azmat.ron",
                false,
                false,
                false,
            ),
        ]))
        .with_node_palette(GraphNodePaletteProjectionData::new(Vec::new()))
    }

    fn asset_status_with_script_root() -> EditorAssetBrowserStatus {
        let source_root = std::env::temp_dir().join("azoth-scripting-ui/scripts");
        EditorAssetBrowserStatus::new(
            "session-1",
            vec![WorkspaceRootData {
                workspace_root_id: 1,
                root_id: 1,
                declared_root_id: "project-scripts".to_owned(),
                owner_id: "project".to_owned(),
                source_root: source_root.to_string_lossy().into_owned(),
                display_name: "Project Scripts".to_owned(),
                portable_key: "project:scripts".to_owned(),
                output_prefix: "scripts".to_owned(),
            }],
            vec![AssetBrowserEntryData {
                entry_id: 1,
                workspace_id: 1,
                asset_guid: "00000000-0000-0000-0000-000000000001".to_owned(),
                root_id: 1,
                source_path: "open_chest.lua".to_owned(),
                schema_type: Some("az.script.Source".to_owned()),
                content_hash: "hash".to_owned(),
                status: AssetBrowserEntryStatus::Clean,
                diagnostics_count: 0,
                latest_job: None,
            }],
            None,
        )
    }

    #[test]
    fn graph_rows_exclude_material_graphs_and_include_script_canvases() {
        let graph = graph_projection_fixture();
        let projection =
            build_scripting_projection(Some(&graph), None, &ScriptingUiState::default());

        assert_eq!(projection.graph_scripts.len(), 1);
        assert_eq!(projection.graph_scripts[0].key, "scripts/ai.azgraph.ron");
        assert_eq!(projection.graph_scripts[0].name, "ai");
        assert_eq!(
            projection.tabs.len(),
            1,
            "only the loaded document is a tab"
        );
        assert_eq!(
            projection.workbench,
            ScriptWorkbenchProjection::Graph {
                document_id: "scripts/ai.azgraph.ron".to_owned(),
                unsaved_changes: false,
            }
        );
        assert_eq!(projection.breadcrumb, vec!["scripts", "ai.azgraph.ron"]);
        assert_eq!(projection.outline.len(), 2, "one node row + one port row");
        assert!(projection.outline_empty_reason.is_empty());
    }

    #[test]
    fn text_selection_builds_file_card_from_registered_script_root() {
        let status = asset_status_with_script_root();
        let state = ScriptingUiState {
            selected_key: "text:1".to_owned(),
        };
        let projection = build_scripting_projection(None, Some(&status), &state);

        let ScriptWorkbenchProjection::Text(row) = &projection.workbench else {
            panic!("expected text workbench, got {:?}", projection.workbench);
        };
        assert_eq!(row.source_path, "open_chest.lua");
        assert_eq!(
            row.source_root,
            std::env::temp_dir()
                .join("azoth-scripting-ui/scripts")
                .to_string_lossy()
        );
        assert!(projection.outline.is_empty());
        assert!(projection.outline_empty_reason.contains("No symbol source"));
    }

    #[test]
    fn no_selection_reports_honest_empty_workbench() {
        let projection = build_scripting_projection(None, None, &ScriptingUiState::default());

        assert!(projection.graph_scripts.is_empty());
        assert!(projection.tabs.is_empty());
        assert_eq!(
            projection.workbench,
            ScriptWorkbenchProjection::Empty {
                message: "Select a script from the left file tree.".to_owned(),
            }
        );
        assert!(projection.outline_empty_reason.contains("Select a script"));
    }

    #[test]
    fn text_source_roots_are_empty_without_a_registered_script_root() {
        let status = EditorAssetBrowserStatus::new(
            "session-1",
            vec![WorkspaceRootData {
                workspace_root_id: 1,
                root_id: 1,
                declared_root_id: "project-assets".to_owned(),
                owner_id: "project".to_owned(),
                source_root: std::env::temp_dir()
                    .join("azoth-scripting-ui/assets")
                    .to_string_lossy()
                    .into_owned(),
                display_name: "Project Assets".to_owned(),
                portable_key: "project:assets".to_owned(),
                output_prefix: "assets".to_owned(),
            }],
            vec![AssetBrowserEntryData {
                entry_id: 1,
                workspace_id: 1,
                asset_guid: "00000000-0000-0000-0000-000000000001".to_owned(),
                root_id: 1,
                source_path: "scripts/open_chest.lua".to_owned(),
                schema_type: None,
                content_hash: "hash".to_owned(),
                status: AssetBrowserEntryStatus::Clean,
                diagnostics_count: 0,
                latest_job: None,
            }],
            None,
        );
        let state = ScriptingUiState {
            selected_key: "text:1".to_owned(),
        };

        // Never a project-wide scan: no registered script root means no text
        // workbench card, even though an entry with a `.lua` path exists
        // under the (unrelated) assets root.
        let projection = build_scripting_projection(None, Some(&status), &state);
        assert_eq!(
            projection.workbench,
            ScriptWorkbenchProjection::Empty {
                message: "Selected script is no longer available.".to_owned(),
            }
        );
    }
}
