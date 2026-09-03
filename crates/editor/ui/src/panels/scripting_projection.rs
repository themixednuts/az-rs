//! Scripting mode projection data.
//!
//! Editor-core owns the scripting boundary: script *canvases* (visual graph
//! documents through the visual-graph controller, same boundary Materials
//! uses for `.azmat.ron`) and script *assets* (text sources under source
//! roots the asset pipeline has actually registered for scripts). It
//! publishes [`EditorScriptingProjection`] for the Scripting mode panels to
//! render; panels dispatch typed `Script*` / graph actions from
//! [`crate::actions`] and never see graph-controller or project-host types.
//!
//! Wave D2 hard scope rule: text script rows are scoped to declared script
//! source roots only — never a project-wide file tree. Today
//! `az_project::ProjectPaths::scripts` (`azoth.toml`) is not wired into the
//! watched source-root list (only `ProjectPaths::assets` is), so
//! [`script_text_source_roots`] legitimately returns nothing until that
//! backend gap closes; this module does not paper over it with path or
//! extension sniffing across the assets root.
//!
//! The text-asset derivation ([`script_text_folders`],
//! [`script_text_row_by_key`]) is a pure function over
//! [`EditorAssetBrowserStatus`], which is already editor-ui-native data.
//! Panels call it directly at render time (always current with the latest
//! asset-browser status) and the editor-core scripting controller reuses the
//! exact same function for the workbench card / breadcrumb — one join, no
//! parallel asset-tree lookups.

use gpui::Global;
use std::collections::BTreeMap;

use crate::panels::asset_browser::{
    AssetBrowserEntryData, EditorAssetBrowserStatus, WorkspaceRootData,
};

/// Root Scripting mode projection global published by editor-core.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditorScriptingProjection {
    /// Script canvases (non-material graph documents) known to the graph
    /// controller's document list.
    pub graph_scripts: Vec<ScriptFileRowProjection>,
    /// Loaded script canvases, shown as workbench tabs (mirrors Materials'
    /// document tabs; text sources have no loaded-buffer concept yet, so
    /// they never appear here — see [`ScriptWorkbenchProjection::Text`]).
    pub tabs: Vec<ScriptDocumentTabProjection>,
    pub selected_key: Option<String>,
    pub breadcrumb: Vec<String>,
    pub outline: Vec<ScriptOutlineRowProjection>,
    /// Why `outline` is empty (no selection, no symbol source for text
    /// scripts, or the selected canvas has no nodes yet).
    pub outline_empty_reason: String,
    pub workbench: ScriptWorkbenchProjection,
}

impl Global for EditorScriptingProjection {}

/// One script canvas row in the file tree / tab strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFileRowProjection {
    pub key: String,
    pub name: String,
    pub source_path: String,
    pub selected: bool,
    pub unsaved_changes: bool,
    pub has_diagnostics: bool,
}

/// One open (loaded) script canvas tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptDocumentTabProjection {
    pub key: String,
    pub label: String,
    pub selected: bool,
    pub unsaved_changes: bool,
}

/// One outline row (graph node, port, or diagnostic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptOutlineRowProjection {
    pub key: String,
    pub label: String,
    pub depth: u32,
    pub kind: ScriptOutlineRowKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptOutlineRowKind {
    Node,
    Port { input: bool },
    Diagnostic,
}

/// Center workbench content, switched by the current selection kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptWorkbenchProjection {
    Empty {
        message: String,
    },
    /// Hosts the shared visual graph canvas (same pattern as
    /// `MaterialWorkbenchPanel`).
    Graph {
        document_id: String,
        unsaved_changes: bool,
    },
    /// No read/write content-load RPC exists for script source files yet
    /// (see `EditorAssetSourcePreview`, which only classifies preview kind,
    /// never loads bytes) — an honest file card with a real
    /// open-in-external-editor action instead of a fake in-app viewer.
    Text(ScriptTextFileRowProjection),
}

impl Default for ScriptWorkbenchProjection {
    fn default() -> Self {
        Self::Empty {
            message: "Select a script from the left file tree.".to_owned(),
        }
    }
}

// ---------------------------------------------------------------------------
// Text script assets: pure derivation over `EditorAssetBrowserStatus`.
// ---------------------------------------------------------------------------

/// One folder group in the text-script tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptFolderProjection {
    pub name: String,
    pub files: Vec<ScriptTextFileRowProjection>,
}

/// One text script asset row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptTextFileRowProjection {
    pub key: String,
    pub name: String,
    pub source_root: String,
    pub source_path: String,
    pub schema_label: String,
    pub status_label: String,
    pub selected: bool,
    pub has_diagnostics: bool,
}

/// Source roots the asset pipeline has registered whose declared purpose is
/// scripts. See the module docs for why this is legitimately empty today.
#[must_use]
pub fn script_text_source_roots(status: &EditorAssetBrowserStatus) -> Vec<&WorkspaceRootData> {
    status
        .roots
        .iter()
        .filter(|root| is_script_source_root(root))
        .collect()
}

fn is_script_source_root(root: &WorkspaceRootData) -> bool {
    let display = root.display_name.to_ascii_lowercase();
    let key = root.portable_key.to_ascii_lowercase();
    display.contains("script") || key.contains("script")
}

/// Text script assets grouped by folder under registered script source
/// roots. Never scans outside those roots (Wave D2 hard scope rule).
#[must_use]
pub fn script_text_folders(
    status: &EditorAssetBrowserStatus,
    selected_key: Option<&str>,
) -> Vec<ScriptFolderProjection> {
    let roots = script_text_source_roots(status);
    if roots.is_empty() {
        return Vec::new();
    }
    let root_by_scan_folder = roots
        .iter()
        .map(|root| (root.root_id, *root))
        .collect::<BTreeMap<_, _>>();

    let mut by_folder = BTreeMap::<String, Vec<ScriptTextFileRowProjection>>::new();
    for entry in &status.entries {
        let Some(root) = root_by_scan_folder.get(&entry.root_id) else {
            continue;
        };
        let folder = script_text_folder_name(root, &entry.source_path);
        by_folder
            .entry(folder)
            .or_default()
            .push(script_text_row(entry, root, selected_key));
    }
    by_folder
        .into_iter()
        .map(|(name, files)| ScriptFolderProjection { name, files })
        .collect()
}

/// Resolve one text script row by key across all registered script roots.
/// Used by the scripting controller to build the workbench card /
/// breadcrumb for a text selection without a second join.
#[must_use]
pub fn script_text_row_by_key(
    status: &EditorAssetBrowserStatus,
    key: &str,
) -> Option<ScriptTextFileRowProjection> {
    script_text_folders(status, Some(key))
        .into_iter()
        .flat_map(|folder| folder.files)
        .find(|row| row.key == key)
}

/// Stable row key for a text script asset.
#[must_use]
pub fn script_text_key(entry: &AssetBrowserEntryData) -> String {
    format!("text:{}", entry.entry_id)
}

fn script_text_folder_name(root: &WorkspaceRootData, source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let mut segments = normalized.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next();
    let root_label = non_empty(&root.display_name, &root.portable_key);
    match (first, segments.next()) {
        (Some(first), Some(_second)) => format!("{root_label}/{first}"),
        _ => root_label.to_owned(),
    }
}

fn script_text_row(
    entry: &AssetBrowserEntryData,
    root: &WorkspaceRootData,
    selected_key: Option<&str>,
) -> ScriptTextFileRowProjection {
    let key = script_text_key(entry);
    ScriptTextFileRowProjection {
        selected: selected_key.is_some_and(|selected| selected == key),
        name: source_stem(&entry.source_path),
        source_root: root.source_root.clone(),
        source_path: entry.source_path.clone(),
        schema_label: entry
            .schema_type
            .clone()
            .unwrap_or_else(|| "unknown".to_owned()),
        status_label: entry.status.label().to_owned(),
        has_diagnostics: entry.diagnostics_count > 0,
        key,
    }
}

fn non_empty<'a>(preferred: &'a str, fallback: &'a str) -> &'a str {
    if preferred.trim().is_empty() {
        fallback
    } else {
        preferred
    }
}

/// "`assets/scripts/ai/open_chest.lua`" -> "`open_chest`".
fn source_stem(path: &str) -> String {
    let mut file_name = path
        .replace('\\', "/")
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .to_owned();
    if let Some(dot) = file_name.find('.') {
        file_name.truncate(dot);
    }
    file_name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::asset_browser::{AssetBrowserEntryStatus, WorkspaceRootData};

    fn source_root(root_id: i64, display_name: &str) -> WorkspaceRootData {
        let slug = display_name.to_ascii_lowercase().replace(' ', "-");
        WorkspaceRootData {
            workspace_root_id: root_id,
            root_id,
            declared_root_id: format!("script-root-{root_id}"),
            owner_id: "project".to_owned(),
            source_root: std::env::temp_dir()
                .join("azoth-scripting-projection/scripts")
                .to_string_lossy()
                .into_owned(),
            display_name: display_name.to_owned(),
            portable_key: format!("project:{slug}"),
            output_prefix: slug,
        }
    }

    fn entry(root_id: i64, source_path: &str, entry_id: i64) -> AssetBrowserEntryData {
        AssetBrowserEntryData {
            entry_id,
            workspace_id: 1,
            asset_guid: format!("00000000-0000-0000-0000-{entry_id:012x}"),
            root_id,
            source_path: source_path.to_owned(),
            schema_type: Some("az.script.Source".to_owned()),
            content_hash: "hash".to_owned(),
            status: AssetBrowserEntryStatus::Clean,
            diagnostics_count: 0,
            latest_job: None,
        }
    }

    #[test]
    fn text_folders_are_empty_without_a_registered_script_root() {
        let status = EditorAssetBrowserStatus::new(
            "session-1",
            vec![source_root(1, "Project Assets")],
            vec![entry(1, "scripts/ai.lua", 1)],
            None,
        );

        assert!(script_text_source_roots(&status).is_empty());
        assert!(script_text_folders(&status, None).is_empty());
    }

    #[test]
    fn text_folders_group_by_first_path_segment_under_a_script_root() {
        let status = EditorAssetBrowserStatus::new(
            "session-1",
            vec![source_root(1, "Project Scripts")],
            vec![
                entry(1, "ai/open_chest.lua", 1),
                entry(1, "ai/enemy.lua", 2),
                entry(1, "startup.lua", 3),
                entry(2, "assets/textures/rock.png", 4),
            ],
            None,
        );

        let folders = script_text_folders(&status, Some("text:2"));
        assert_eq!(folders.len(), 2);

        let ai_folder = folders
            .iter()
            .find(|folder| folder.name == "Project Scripts/ai")
            .expect("ai folder");
        assert_eq!(ai_folder.files.len(), 2);
        let enemy = ai_folder
            .files
            .iter()
            .find(|file| file.key == "text:2")
            .expect("enemy row");
        assert!(enemy.selected);
        assert_eq!(enemy.name, "enemy");

        let root_folder = folders
            .iter()
            .find(|folder| folder.name == "Project Scripts")
            .expect("root folder");
        assert_eq!(root_folder.files.len(), 1);
        assert_eq!(root_folder.files[0].name, "startup");
    }

    #[test]
    fn text_row_by_key_resolves_across_folders() {
        let status = EditorAssetBrowserStatus::new(
            "session-1",
            vec![source_root(1, "Scripts")],
            vec![entry(1, "ai/open_chest.lua", 7)],
            None,
        );

        let row = script_text_row_by_key(&status, "text:7").expect("row");
        assert_eq!(row.source_path, "ai/open_chest.lua");
        assert_eq!(row.schema_label, "az.script.Source");
        assert_eq!(row.status_label, "clean");

        assert!(script_text_row_by_key(&status, "text:404").is_none());
    }
}
