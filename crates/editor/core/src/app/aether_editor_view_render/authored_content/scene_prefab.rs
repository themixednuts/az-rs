//! Scene, level, prefab, and authored-document projections.

use std::path::Path;

use az_editor_inspector::ReflectedEntityInspection;
use az_editor_ui::EditorSceneToolState;
use az_editor_ui::panels::{
    AuthoredDocumentOutlineData, AuthoredLayerRow, AuthoredObjectOutlineData, EditorActiveLevel,
    EditorAssetBrowserStatus, EditorAuthoredOutline, EditorCreatableAuthoredSchemas,
    EditorLayerVisibility, EditorReflectedSelectionState, EditorRuntimeStatus,
    active_level_prefab_documents, authored_layer_rows, is_scene_document,
    is_scene_document_schema,
};
use az_editor_ui::type_iconography::{EditorTypeKind, asset_kind};
use gpui::{AppContext, Context, Rgba};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherStyle};
use crate::app::aether_editor_model::AetherEditorState;
use crate::app::aether_editor_view::AetherEditorView;

use super::super::diagnostics_preview::runtime_play_icon;
use super::super::presentation::{
    apply_expandable_item_state, hsla_css, non_empty_string_or, path_stem_label, plural_count,
    rgb_u32_css, set_item_style,
};
use super::inspector::{prefab_override_item, reflected_node_label};
use super::schema_presentation::{schema_color, schema_display_label, schema_icon};

impl AetherEditorView {
    pub(crate) fn hierarchy_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if let Some(outline) = cx.try_global::<EditorAuthoredOutline>() {
            return self.hierarchy_rows_from_outline(
                outline,
                cx.try_global::<EditorActiveLevel>()
                    .and_then(|active| active.document_id.as_deref()),
            );
        }
        Vec::new()
    }
    pub(crate) fn layer_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(outline) = cx.try_global::<EditorAuthoredOutline>() else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        let active_document_id = active_authored_document_id(
            cx.try_global::<EditorReflectedSelectionState>()
                .and_then(EditorReflectedSelectionState::current),
            outline,
        );
        authored_layer_rows(outline, cx.try_global::<EditorLayerVisibility>())
            .into_iter()
            .map(|row| authored_layer_item(row, active_document_id.as_deref(), &theme))
            .collect()
    }
    pub(crate) fn level_actions(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        level_action_items(
            cx.try_global::<EditorCreatableAuthoredSchemas>(),
            active_level_document_id(
                cx.try_global::<EditorActiveLevel>(),
                cx.try_global::<EditorAuthoredOutline>(),
            )
            .is_some(),
        )
    }
    pub(crate) fn level_items(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(outline) = cx.try_global::<EditorAuthoredOutline>() else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        level_items_from_outline(
            outline,
            active_level_document_id(cx.try_global::<EditorActiveLevel>(), Some(outline))
                .as_deref(),
            &theme,
        )
    }
    pub(crate) fn prefab_actions(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        else {
            return Vec::new();
        };
        let source = inspection.selection.source_path.clone();
        if source.trim().is_empty() {
            return Vec::new();
        }
        vec![AetherItem {
            kind: "prefab-source-document".to_owned(),
            key: source.clone(),
            label: "Open Source".to_owned(),
            icon: "open_in_new".to_owned(),
            ..AetherItem::default()
        }]
    }
    pub(crate) fn prefab_overrides(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return self.prefab_overrides_from_inspection(inspection);
        }
        Vec::new()
    }
    pub(crate) fn prefab_tree(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return self.prefab_tree_from_inspection(inspection);
        }
        if let Some(outline) = cx.try_global::<EditorAuthoredOutline>() {
            return self.prefab_tree_from_outline(outline);
        }
        Vec::new()
    }
    pub(crate) fn cleanup_authored_source_path_moves(&mut self, cx: &mut Context<Self>) {
        let Some(outline) = cx.try_global::<EditorAuthoredOutline>() else {
            return;
        };
        if self
            .state
            .clear_resolved_authored_source_path_moves(outline)
        {
            tracing::info!(
                target: "az_editor::aether_ui",
                "cleared resolved Aether authored source path rename remaps"
            );
        }
    }
    pub(crate) fn is_selected_prefab_instance(&self, cx: &mut Context<Self>) -> bool {
        cx.try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
            .is_some_and(|inspection| !inspection.overrides.is_empty())
    }
    pub(crate) fn has_authored_selection(&self, cx: &mut Context<Self>) -> bool {
        cx.try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
            .is_some()
    }
    pub(crate) fn ent_is_prefab(&self, cx: &mut Context<Self>) -> bool {
        self.is_selected_prefab_instance(cx)
    }
    pub(crate) fn has_sublevels(&self) -> bool {
        false
    }
    pub(crate) fn level_dirty(&self, cx: &mut Context<Self>) -> bool {
        active_level_document(
            cx.try_global::<EditorActiveLevel>(),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .is_some_and(|document| document.unsaved_changes)
    }
    pub(crate) fn level_open(&self) -> bool {
        self.bool_value("levelOpen")
    }
    pub(crate) fn mode_animation(&self) -> bool {
        self.bool_value("modeAnimation")
    }
    pub(crate) fn mode_materials(&self) -> bool {
        self.bool_value("modeMaterials")
    }
    pub(crate) fn mode_scene(&self) -> bool {
        self.bool_value("modeScene")
    }
    pub(crate) fn mode_scripting(&self) -> bool {
        self.bool_value("modeScripting")
    }
    pub(crate) fn mode_sequencer(&self) -> bool {
        self.bool_value("modeSequencer")
    }
    pub(crate) fn prefab_overridden(&self, cx: &mut Context<Self>) -> bool {
        !self.prefab_overrides(cx).is_empty()
    }
    pub(crate) fn right_details(&self, cx: &mut Context<Self>) -> bool {
        self.has_authored_selection(cx)
            && (self.state.workspace_projection().right_tab != "prefab"
                || !self.is_selected_prefab_instance(cx))
    }
    pub(crate) fn right_empty(&self, cx: &mut Context<Self>) -> bool {
        !self.has_authored_selection(cx)
    }
    pub(crate) fn right_prefab(&self, cx: &mut Context<Self>) -> bool {
        self.state.workspace_projection().right_tab == "prefab"
            && self.is_selected_prefab_instance(cx)
    }
    pub(crate) fn active_layer(&self, cx: &mut Context<Self>) -> String {
        let Some(outline) = cx.try_global::<EditorAuthoredOutline>() else {
            return "No active layer".to_owned();
        };
        active_authored_document(
            cx.try_global::<EditorReflectedSelectionState>()
                .and_then(EditorReflectedSelectionState::current),
            Some(outline),
        )
        .filter(|document| is_scene_document(document))
        .map(|document| {
            az_editor_ui::panels::layers::layer_name(&document.source_path, &document.document_id)
        })
        .unwrap_or_else(|| "No active layer".to_owned())
    }
    pub(crate) fn active_material_name(&self, cx: &mut Context<Self>) -> String {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            if inspection.components.iter().any(|component| {
                component
                    .component
                    .type_path
                    .to_ascii_lowercase()
                    .contains("material")
            }) {
                return authored_object_name(inspection);
            }
        }
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>()
            && let Some(entry) = status.entries.iter().find(|entry| {
                entry
                    .schema_type
                    .as_deref()
                    .is_some_and(|schema| asset_kind(schema, schema) == EditorTypeKind::Material)
            })
        {
            return path_stem_label(&entry.source_path);
        }
        "Material".to_owned()
    }
    pub(crate) fn active_level_icon(&self, cx: &mut Context<Self>) -> String {
        active_level_document(
            cx.try_global::<EditorActiveLevel>(),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .map(|document| schema_icon(&document.schema_type).to_owned())
        .unwrap_or_else(|| "widgets".to_owned())
    }
    pub(crate) fn active_level_name(&self, cx: &mut Context<Self>) -> String {
        active_level_document(
            cx.try_global::<EditorActiveLevel>(),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .map(level_document_name)
        .unwrap_or_else(|| "No Level".to_owned())
    }
    pub(crate) fn active_level_color(&self, cx: &mut Context<Self>) -> String {
        let theme = cx.theme().clone();
        if active_level_document_id(
            cx.try_global::<EditorActiveLevel>(),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .is_some()
        {
            hsla_css(theme.accent)
        } else {
            hsla_css(theme.muted_foreground)
        }
    }
    pub(crate) fn angle_val(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default()
            .angle_degrees_label()
    }
    pub(crate) fn ent_icon(&self) -> String {
        self.string_value("entIcon")
    }
    pub(crate) fn ent_icon_color(&self) -> String {
        self.string_value("entIconColor")
    }
    pub(crate) fn ent_kind(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
            .and_then(|inspection| inspection.components.first())
            .map(|component| schema_display_label(&component.component.type_path))
            .unwrap_or_else(|| self.string_value("entKind"))
    }
    pub(crate) fn ent_layer(&self, cx: &mut Context<Self>) -> String {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return inspection.selection.source_path.clone();
        }
        self.string_value("entLayer")
    }
    pub(crate) fn ent_name(&self, cx: &mut Context<Self>) -> String {
        let selection = self.state.asset_selection();
        if !selection.name.is_empty() {
            return selection.name.to_owned();
        }
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return authored_object_name(inspection);
        }
        active_level_document(
            cx.try_global::<EditorActiveLevel>(),
            cx.try_global::<EditorAuthoredOutline>(),
        )
        .map(level_document_name)
        .unwrap_or_else(|| "No selection".to_owned())
    }
    pub(crate) fn ent_tag(&self, cx: &mut Context<Self>) -> String {
        let selection = self.state.asset_selection();
        if !selection.schema_type.is_empty() {
            return schema_display_label(selection.schema_type);
        }
        String::new()
    }
    pub(crate) fn entity_count_label(&self, cx: &mut Context<Self>) -> String {
        if let Some(outline) = cx.try_global::<EditorAuthoredOutline>() {
            let (documents, objects) = scene_document_counts(outline);
            return format!(
                "{} · {}",
                plural_count(documents, "document"),
                plural_count(objects, "object")
            );
        }
        self.string_value("entityCountLabel")
    }
    pub(crate) fn layer_count(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorAuthoredOutline>()
            .map(|outline| scene_document_counts(outline).0.to_string())
            .unwrap_or_else(|| "0".to_owned())
    }
    pub(crate) fn level_count_label(&self, cx: &mut Context<Self>) -> String {
        let count = cx
            .try_global::<EditorAuthoredOutline>()
            .map(level_documents)
            .map(|levels| levels.len())
            .unwrap_or(0);
        plural_count(count, "level")
    }
    pub(crate) fn play_icon(&self, cx: &mut Context<Self>) -> String {
        runtime_play_icon(cx.try_global::<EditorRuntimeStatus>()).to_owned()
    }
    pub(crate) fn prefab_name(&self, cx: &mut Context<Self>) -> String {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return authored_object_name(inspection);
        }
        self.string_value("prefabName")
    }
    pub(crate) fn prefab_override_count(&self, cx: &mut Context<Self>) -> String {
        self.prefab_overrides(cx).len().to_string()
    }
    pub(crate) fn prefab_path(&self, cx: &mut Context<Self>) -> String {
        if let Some(inspection) = cx
            .try_global::<EditorReflectedSelectionState>()
            .and_then(EditorReflectedSelectionState::current)
        {
            return inspection.selection.source_path.clone();
        }
        self.string_value("prefabPath")
    }
    pub(crate) fn snap_val(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default()
            .grid_step_label()
    }
    fn hierarchy_rows_from_outline(
        &self,
        outline: &EditorAuthoredOutline,
        active_level_document_id: Option<&str>,
    ) -> Vec<AetherItem> {
        hierarchy_rows_from_outline_state(&self.state, outline, active_level_document_id)
    }

    fn prefab_tree_from_inspection(
        &self,
        inspection: &ReflectedEntityInspection,
    ) -> Vec<AetherItem> {
        let mut rows = Vec::new();
        rows.push(AetherItem {
            key: inspection.selection.entity_alias.clone(),
            name: authored_object_name(inspection),
            icon: "deployed_code".to_owned(),
            icon_color: "#4188e0".to_owned(),
            kind: "root".to_owned(),
            depth: 0,
            row_style: prefab_tree_row_style(0),
            ..AetherItem::default()
        });
        rows.extend(inspection.components.iter().map(|component| {
            AetherItem {
                key: component.component.type_path.clone(),
                name: component.model.type_label.clone(),
                icon: component
                    .model
                    .icon
                    .clone()
                    .unwrap_or_else(|| "extension".to_owned()),
                icon_color: schema_color(&component.component.type_path).to_owned(),
                kind: "component".to_owned(),
                depth: 1,
                row_style: prefab_tree_row_style(1),
                ..AetherItem::default()
            }
        }));
        rows
    }

    fn prefab_tree_from_outline(&self, outline: &EditorAuthoredOutline) -> Vec<AetherItem> {
        let mut rows = Vec::new();
        for document in &outline.data.documents {
            let open = self
                .state
                .item_expanded(&document.document_id, !document.objects.is_empty());
            let mut item = AetherItem {
                key: document.document_id.clone(),
                name: non_empty_string_or(
                    path_stem_label(&document.source_path),
                    &document.schema_type,
                ),
                icon: schema_icon(&document.schema_type).to_owned(),
                icon_color: schema_color(&document.schema_type).to_owned(),
                kind: "document".to_owned(),
                depth: 0,
                has_children: !document.objects.is_empty(),
                caret: if document.objects.is_empty() {
                    String::new()
                } else {
                    "arrow_drop_down".to_owned()
                },
                row_style: prefab_tree_row_style(0),
                ..AetherItem::default()
            };
            apply_expandable_item_state(&mut item, open);
            rows.push(item);
            if open {
                rows.extend(document.objects.iter().map(|object| AetherItem {
                    key: object.object_id.clone(),
                    name: object.object_id.clone(),
                    icon: schema_icon(&object.schema_type).to_owned(),
                    icon_color: schema_color(&object.schema_type).to_owned(),
                    kind: schema_display_label(&object.schema_type),
                    depth: 1,
                    row_style: prefab_tree_row_style(1),
                    ..AetherItem::default()
                }));
            }
        }
        rows
    }

    fn prefab_overrides_from_inspection(
        &self,
        inspection: &ReflectedEntityInspection,
    ) -> Vec<AetherItem> {
        inspection
            .overrides
            .iter()
            .enumerate()
            .map(|(index, operation)| prefab_override_item(index, operation))
            .collect()
    }
}

fn hierarchy_row_style(depth: u32, selected: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "5px".to_owned()),
        ("height", "23px".to_owned()),
        ("paddingLeft", format!("{}px", 6 + depth * 13)),
        ("paddingRight", "8px".to_owned()),
        ("cursor", "default".to_owned()),
        ("fontSize", "11.5px".to_owned()),
        (
            "borderLeft",
            if selected {
                "2px solid #4188e0"
            } else {
                "2px solid transparent"
            }
            .to_owned(),
        ),
        (
            "background",
            if selected {
                "rgba(65,136,224,0.13)"
            } else {
                "transparent"
            }
            .to_owned(),
        ),
    ])
}

fn hierarchy_caret_style(show: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("fontSize", "18px".to_owned()),
        ("width", "15px".to_owned()),
        ("flex", "0 0 15px".to_owned()),
        ("color", "#6b7280".to_owned()),
        ("cursor", "default".to_owned()),
        ("opacity", if show { "1" } else { "0" }.to_owned()),
    ])
}

fn hierarchy_icon_style(color: &str) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("fontSize", "15px".to_owned()),
        ("color", color.to_owned()),
        ("flex", "0 0 auto".to_owned()),
    ])
}

pub(super) fn hierarchy_name_style(selected: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("flex", "1 1 auto".to_owned()),
        ("overflow", "hidden".to_owned()),
        ("textOverflow", "ellipsis".to_owned()),
        ("whiteSpace", "nowrap".to_owned()),
        (
            "color",
            if selected { "#ffffff" } else { "#cfd4db" }.to_owned(),
        ),
        (
            "fontWeight",
            if selected { "600" } else { "400" }.to_owned(),
        ),
    ])
}

fn hierarchy_tag_style() -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("fontSize", "9px".to_owned()),
        ("color", "#7c838f".to_owned()),
        ("background", "#23272e".to_owned()),
        ("padding", "1px 5px".to_owned()),
        ("borderRadius", "3px".to_owned()),
        ("flex", "0 0 auto".to_owned()),
    ])
}

fn prefab_tree_row_style(depth: u32) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "7px".to_owned()),
        ("height", "25px".to_owned()),
        ("cursor", "default".to_owned()),
        ("paddingLeft", format!("{}px", 12 + depth * 16)),
        ("paddingRight", "12px".to_owned()),
    ])
}

pub(crate) fn active_authored_document_id(
    inspection: Option<&ReflectedEntityInspection>,
    outline: &EditorAuthoredOutline,
) -> Option<String> {
    inspection
        .map(|inspection| inspection.selection.source_path.clone())
        .or_else(|| {
            active_authored_document(None, Some(outline))
                .map(|document| document.document_id.clone())
        })
}

pub(crate) fn active_authored_document<'a>(
    inspection: Option<&ReflectedEntityInspection>,
    outline: Option<&'a EditorAuthoredOutline>,
) -> Option<&'a AuthoredDocumentOutlineData> {
    let outline = outline?;
    if let Some(inspection) = inspection {
        return outline.data.documents.iter().find(|document| {
            document.document_id == inspection.selection.source_path
                || document.source_path == inspection.selection.source_path
        });
    }
    outline
        .data
        .documents
        .iter()
        .find(|document| document.objects.iter().any(|object| object.selected))
}

fn scene_documents(
    outline: &EditorAuthoredOutline,
) -> impl Iterator<Item = &AuthoredDocumentOutlineData> {
    outline
        .data
        .documents
        .iter()
        .filter(|document| is_scene_document(document))
}

pub(crate) fn scene_document_counts(outline: &EditorAuthoredOutline) -> (usize, u32) {
    scene_documents(outline).fold((0, 0), |(documents, objects), document| {
        (documents + 1, objects + document.object_count)
    })
}

pub(crate) fn level_documents(
    outline: &EditorAuthoredOutline,
) -> Vec<&AuthoredDocumentOutlineData> {
    scene_documents(outline).collect()
}

fn active_level_document_id(
    active_level: Option<&EditorActiveLevel>,
    outline: Option<&EditorAuthoredOutline>,
) -> Option<String> {
    active_level_document(active_level, outline).map(|document| document.document_id.clone())
}

fn active_level_document<'a>(
    active_level: Option<&EditorActiveLevel>,
    outline: Option<&'a EditorAuthoredOutline>,
) -> Option<&'a AuthoredDocumentOutlineData> {
    let document_id = active_level?.document_id.as_deref()?;
    outline?.data.documents.iter().find(|document| {
        document.document_id == document_id
            && document.loaded
            && document.valid
            && is_scene_document(document)
    })
}

fn level_document_name(document: &AuthoredDocumentOutlineData) -> String {
    if let Some(name) = document
        .objects
        .iter()
        .find(|object| object.schema_type == az_prefab::SCENE_SOURCE_TYPE)
        .and_then(|object| object.display_name.as_deref())
        .filter(|name| !name.trim().is_empty())
    {
        return name.to_owned();
    }
    authored_document_label(&document.source_path, &document.schema_type)
}

fn authored_layer_item(
    row: AuthoredLayerRow,
    active_document_id: Option<&str>,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let active = active_document_id == Some(row.document_id.as_str());
    let visible = !row.hidden;
    let mut item = AetherItem {
        key: row.document_id,
        kind: "layer".to_owned(),
        name: row.name,
        count: row.count.to_string(),
        active,
        has_tag: row.unsaved,
        tag: if row.unsaved {
            "modified".to_owned()
        } else {
            String::new()
        },
        lock_icon: if row.locked { "lock" } else { "lock_open" }.to_owned(),
        vis_icon: if visible {
            "visibility"
        } else {
            "visibility_off"
        }
        .to_owned(),
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", layer_row_style(active, theme));
    set_item_style(&mut item, "dotStyle", layer_dot_style(row.color, theme));
    set_item_style(
        &mut item,
        "nameStyle",
        layer_name_style(active, visible, theme),
    );
    set_item_style(&mut item, "lockStyle", layer_lock_style(row.locked, theme));
    set_item_style(
        &mut item,
        "visStyle",
        layer_visibility_style(visible, theme),
    );
    item
}

pub(crate) fn level_items_from_outline(
    outline: &EditorAuthoredOutline,
    active_document_id: Option<&str>,
    theme: &gpui_component::theme::Theme,
) -> Vec<AetherItem> {
    level_documents(outline)
        .into_iter()
        .map(|document| {
            let active = active_document_id == Some(document.document_id.as_str());
            let mut item = AetherItem {
                key: document.document_id.clone(),
                id: document.document_id.clone(),
                kind: "level".to_owned(),
                name: level_document_name(document),
                icon: schema_icon(&document.schema_type).to_owned(),
                color: hsla_css(if active {
                    theme.accent
                } else {
                    theme.muted_foreground
                }),
                meta: level_document_meta(document),
                active,
                // Selecting an unloaded document is what asks project-host to
                // load it. Decodable invalid drafts then expose their
                // schema-derived inspector fields for repair. Loaded/valid
                // still controls whether the document becomes the active level.
                disabled: false,
                level_dirty: document.unsaved_changes,
                ..AetherItem::default()
            };
            set_item_style(&mut item, "style", level_row_style(active, theme));
            set_item_style(
                &mut item,
                "iconTileStyle",
                level_icon_tile_style(active, theme),
            );
            item
        })
        .collect()
}

pub(crate) fn level_document_meta(document: &AuthoredDocumentOutlineData) -> String {
    format!(
        "{} · {}",
        non_empty_string_or(&document.source_path, &document.document_id),
        plural_count(document.object_count, "object")
    )
}

pub(crate) fn level_action_items(
    schemas: Option<&EditorCreatableAuthoredSchemas>,
    has_active_level: bool,
) -> Vec<AetherItem> {
    let mut actions = Vec::new();
    if schemas.is_some_and(|schemas| {
        schemas
            .schemas
            .iter()
            .any(|schema| is_scene_document_schema(&schema.schema_type))
    }) {
        actions.push(level_action_item("new-level", "New Level...", "add", ""));
    }
    if has_active_level {
        actions.push(level_action_item(
            "save-level",
            "Save Level",
            "save",
            "Ctrl+S",
        ));
    }
    actions.push(level_action_item(
        "refresh-levels",
        "Refresh",
        "refresh",
        "",
    ));
    actions
}

fn level_action_item(key: &str, label: &str, icon: &str, shortcut: &str) -> AetherItem {
    AetherItem {
        key: key.to_owned(),
        kind: "level-action".to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        shortcut: shortcut.to_owned(),
        ..AetherItem::default()
    }
}

fn layer_row_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        (
            "background",
            hsla_css(if active {
                theme.accent.opacity(0.13)
            } else {
                theme.transparent
            }),
        ),
        (
            "borderLeft",
            if active {
                format!("2px solid {}", hsla_css(theme.accent))
            } else {
                "2px solid transparent".to_owned()
            },
        ),
    ])
}

fn layer_dot_style(color: u32, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("background", rgb_u32_css(color)),
        ("borderColor", hsla_css(theme.border)),
    ])
}

fn layer_name_style(
    active: bool,
    visible: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("flex", "1 1 auto".to_owned()),
        ("overflow", "hidden".to_owned()),
        ("textOverflow", "ellipsis".to_owned()),
        ("whiteSpace", "nowrap".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.foreground
            } else if visible {
                theme.sidebar_foreground
            } else {
                theme.muted_foreground
            }),
        ),
        ("fontWeight", if active { "600" } else { "400" }.to_owned()),
    ])
}

fn layer_lock_style(locked: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "color",
        hsla_css(if locked {
            theme.warning
        } else {
            theme.muted_foreground
        }),
    )])
}

fn layer_visibility_style(visible: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "color",
        hsla_css(if visible {
            theme.muted_foreground
        } else {
            theme.warning
        }),
    )])
}

fn level_row_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "background",
        hsla_css(if active {
            theme.accent.opacity(0.13)
        } else {
            theme.transparent
        }),
    )])
}

fn level_icon_tile_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "background",
        hsla_css(if active {
            theme.accent.opacity(0.16)
        } else {
            theme.secondary
        }),
    )])
}

pub(crate) fn hierarchy_rows_from_outline_state(
    state: &AetherEditorState,
    outline: &EditorAuthoredOutline,
    active_level_document_id: Option<&str>,
) -> Vec<AetherItem> {
    let mut rows = Vec::new();
    let Some(active_level_document_id) = active_level_document_id else {
        return rows;
    };
    let Some(level) = outline.data.documents.iter().find(|document| {
        document.document_id == active_level_document_id
            && document.loaded
            && document.valid
            && is_scene_document(document)
    }) else {
        return rows;
    };
    let root_prefab = active_level_prefab_documents(&outline.data, Some(active_level_document_id))
        .into_iter()
        .next();
    let open = state.item_expanded(&level.document_id, root_prefab.is_some());
    let mut level_row = authored_document_hierarchy_row(level);
    apply_expandable_item_state(&mut level_row, open);
    rows.push(level_row);
    if open {
        if let Some(root_prefab) = root_prefab {
            for object in &root_prefab.objects {
                rows.push(authored_object_hierarchy_row(root_prefab, object));
            }
        }
    }
    rows
}

fn authored_document_hierarchy_row(document: &AuthoredDocumentOutlineData) -> AetherItem {
    let name = non_empty_string_or(
        path_stem_label(&document.source_path),
        &document.schema_type,
    );
    let color = schema_color(&document.schema_type);
    let selected = document.objects.iter().any(|object| object.selected);
    let mut item = AetherItem {
        key: document.document_id.clone(),
        id: document.document_id.clone(),
        name,
        depth: 0,
        has_children: !document.objects.is_empty(),
        caret: if document.objects.is_empty() {
            String::new()
        } else {
            "arrow_drop_down".to_owned()
        },
        icon: schema_icon(&document.schema_type).to_owned(),
        has_tag: document.unsaved_changes,
        tag: if document.unsaved_changes {
            "modified".to_owned()
        } else {
            String::new()
        },
        selected,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", hierarchy_row_style(0, selected));
    set_item_style(
        &mut item,
        "caretStyle",
        hierarchy_caret_style(!document.objects.is_empty()),
    );
    set_item_style(&mut item, "iconStyle", hierarchy_icon_style(color));
    set_item_style(&mut item, "nameStyle", hierarchy_name_style(selected));
    set_item_style(&mut item, "tagStyle", hierarchy_tag_style());
    item
}

fn authored_object_hierarchy_row(
    document: &AuthoredDocumentOutlineData,
    object: &AuthoredObjectOutlineData,
) -> AetherItem {
    let color = schema_color(&object.schema_type);
    let mut item = AetherItem {
        key: format!("{}:{}", document.document_id, object.object_id),
        id: object.object_id.clone(),
        name: object.object_id.clone(),
        depth: 1,
        has_children: false,
        caret: String::new(),
        icon: schema_icon(&object.schema_type).to_owned(),
        has_tag: true,
        tag: schema_display_label(&object.schema_type),
        selected: object.selected,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", hierarchy_row_style(1, object.selected));
    set_item_style(&mut item, "caretStyle", hierarchy_caret_style(false));
    set_item_style(&mut item, "iconStyle", hierarchy_icon_style(color));
    set_item_style(
        &mut item,
        "nameStyle",
        hierarchy_name_style(object.selected),
    );
    set_item_style(&mut item, "tagStyle", hierarchy_tag_style());
    item
}

fn authored_object_name(inspection: &ReflectedEntityInspection) -> String {
    for field in inspection
        .components
        .iter()
        .flat_map(|component| &component.model.fields)
    {
        if field.name.eq_ignore_ascii_case("name") || field.label.eq_ignore_ascii_case("name") {
            let value = reflected_node_label(&field.value);
            if !value.trim().is_empty() {
                return value;
            }
        }
    }
    inspection.selection.entity_alias.clone()
}

fn active_outline_document(
    outline: &EditorAuthoredOutline,
) -> Option<&AuthoredDocumentOutlineData> {
    outline
        .data
        .documents
        .iter()
        .find(|document| document.objects.iter().any(|object| object.selected))
        .or_else(|| {
            outline
                .data
                .documents
                .iter()
                .find(|document| document.loaded)
        })
        .or_else(|| outline.data.documents.first())
}

fn authored_document_label(path: &str, schema_type: &str) -> String {
    let schema_label = schema_display_label(schema_type);
    az_editor_ui::naming::document_display_name(path, &schema_label).into_owned()
}
