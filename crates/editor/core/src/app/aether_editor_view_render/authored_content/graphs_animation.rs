//! Graph, material, animation, and scene-tool projections.

use std::path::Path;

use az_editor_ui::panels::{
    AssetBrowserEntryData, AssetBrowserJobStatus, EditorAssetBrowserStatus,
    EditorGraphDocumentProjection, EditorMannequinPreview,
};
use az_editor_ui::type_iconography::{EditorTypeKind, asset_kind};
use az_editor_ui::{
    EditorScenePivot, EditorSceneToolKind, EditorSceneToolState, EditorSceneTransformSpace,
};
use gpui::{AppContext, Context, Rgba, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherStyle};
use crate::app::aether_editor_model::trace_aether_ui_interaction;
use crate::app::aether_editor_view::AetherEditorView;

use super::super::presentation::{
    hsla_css, non_empty_string_or, path_stem_label, set_item_style, toolbar_action_item,
};
use super::scene_prefab::hierarchy_name_style;

impl AetherEditorView {
    pub(crate) fn modes(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut modes = aether_mode_items();
        self.apply_collection_state("modes", &mut modes, &theme);
        modes
    }
    pub(crate) fn pivot_opts(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let state = cx
            .try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default();
        let theme = cx.theme().clone();
        EditorScenePivot::ALL
            .into_iter()
            .map(|pivot| scene_pivot_item(pivot, state.pivot == pivot, &theme))
            .collect()
    }
    pub(crate) fn material_tool_items(&self) -> Vec<AetherItem> {
        vec![
            toolbar_action_item(
                "disabled",
                "add-node",
                "Add Node",
                "add_box",
                "Use the graph palette or canvas menu to choose a typed node",
            ),
            toolbar_action_item(
                "graph-auto-layout",
                "auto-layout",
                "Auto Layout",
                "account_tree",
                "Auto-layout the active material graph",
            ),
            toolbar_action_item(
                "disabled",
                "fit",
                "Fit",
                "fit_screen",
                "Fit is unavailable until graph viewport framing is exposed",
            ),
        ]
    }
    pub(crate) fn script_tool_items(&self) -> Vec<AetherItem> {
        vec![
            toolbar_action_item(
                "disabled",
                "find",
                "Find",
                "search",
                "Find is unavailable until graph search state is exposed",
            ),
            toolbar_action_item(
                "graph-create-comment",
                "comment",
                "Comment",
                "comment",
                "Add a comment to the active script graph",
            ),
            toolbar_action_item(
                "graph-auto-layout",
                "format",
                "Format",
                "format_align_left",
                "Format the active script graph with auto layout",
            ),
        ]
    }
    pub(crate) fn sequencer_tool_items(&self) -> Vec<AetherItem> {
        vec![
            toolbar_action_item(
                "disabled",
                "add-track",
                "Add Track",
                "add",
                "Track authoring is not exposed by the current sequencer backend",
            ),
            toolbar_action_item(
                "disabled",
                "add-key",
                "Add Key",
                "fiber_manual_record",
                "Key authoring is not exposed by the current sequencer backend",
            ),
            toolbar_action_item(
                "disabled",
                "curves",
                "Curves",
                "show_chart",
                "Curve editing is not exposed by the current sequencer backend",
            ),
        ]
    }
    pub(crate) fn animation_tool_items(&self) -> Vec<AetherItem> {
        vec![
            toolbar_action_item(
                "disabled",
                "add-parameter",
                "Add Parameter",
                "add",
                "Parameter authoring is not exposed by the animation backend",
            ),
            toolbar_action_item(
                "disabled",
                "add-node",
                "Add Node",
                "account_tree",
                "Use the graph palette to choose a typed animation node",
            ),
            toolbar_action_item(
                "disabled",
                "record",
                "Record",
                "fiber_manual_record",
                "Animation recording is not exposed by the runtime backend",
            ),
        ]
    }
    pub(crate) fn animation_toolbar_label(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorGraphDocumentProjection>()
            .and_then(|projection| projection.document.as_ref())
            .filter(|document| document.graph_type.to_ascii_lowercase().contains("anim"))
            .map(|document| path_stem_label(&document.document_id))
            .filter(|label| !label.is_empty())
            .unwrap_or_else(|| "No animation graph".to_owned())
    }
    pub(crate) fn space_opts(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let state = cx
            .try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default();
        let theme = cx.theme().clone();
        EditorSceneTransformSpace::ALL
            .into_iter()
            .map(|space| scene_space_item(space, state.space == space, &theme))
            .collect()
    }
    pub(crate) fn tools(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let state = cx
            .try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default();
        let theme = cx.theme().clone();
        EditorSceneToolKind::ALL
            .into_iter()
            .map(|tool| scene_tool_item(tool, state.tool == tool, &theme))
            .collect()
    }
    pub(crate) fn set_mode_with_window(
        &mut self,
        mode: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mode.is_empty() || self.state.workspace_projection().mode == mode {
            return;
        }
        self.persist_workspace_state(true, cx);
        if !self.state.set_mode_state(mode) {
            return;
        }
        crate::workspace::dock::apply_workspace_layout_for_mode(&self.dock_area, mode, window, cx);
        crate::ui_state_persistence::set_last_mode(mode, true, cx);
        self.notify_mode_regions(cx);
    }

    pub(crate) fn route_material_source_selection(
        &mut self,
        action: &crate::actions::SelectAuthoredDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.propagate();
        if crate::source_navigation::material_source_kind(&action.document_id).is_none() {
            return;
        }
        self.set_mode_with_window("materials", window, cx);
        crate::materials_ui::sync_material_selection_from_authored(cx, action.document_id.clone());
    }
    /// Asset-browser → Scripting routing: activating a graph document that
    /// is not a material graph switches the workspace into Scripting mode
    /// and records/opens the selection. No dedicated script graph type is
    /// registered anywhere yet, so "not a material graph" is the only real
    /// signal available (see `crate::scripting_ui` module docs); text script
    /// assets have no authored-document activation path yet either. The
    /// action always propagates so the graph controller still loads the
    /// document.
    pub(crate) fn route_script_source_selection(
        &mut self,
        action: &crate::actions::SelectAuthoredDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.propagate();
        if crate::source_navigation::material_source_kind(&action.document_id).is_some() {
            return;
        }
        let is_script_graph = cx
            .try_global::<EditorGraphDocumentProjection>()
            .is_some_and(|graph| {
                graph.graph_documents.documents.iter().any(|entry| {
                    (entry.document_id == action.document_id
                        || entry.source_path == action.document_id)
                        && entry.graph_type != az_material::MATERIAL_GRAPH_ASSET_TYPE_HINT
                })
            });
        if !is_script_graph {
            return;
        }
        self.set_mode_with_window("scripting", window, cx);
        crate::scripting_ui::sync_script_selection_from_authored(cx, action.document_id.clone());
    }
    pub(crate) fn show_graph_workspace(
        &mut self,
        _action: &crate::actions::ToggleGraphPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Graph is a center workbench contribution, not a permanent mode body.
        self.set_mode_with_window("scene", window, cx);
        crate::workspace::dock::open_center_workbench(
            &self.dock_area,
            crate::workspace::layout::visual_graph_workbench(),
            window,
            cx,
        );
        cx.notify();
    }

    pub(crate) fn on_play_click<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace_aether_ui_interaction("transport.play", "dispatch=LaunchEditorWorld");
        window.dispatch_action(Box::new(az_editor_ui::actions::LaunchEditorWorld), cx);
        cx.stop_propagation();
    }
    pub(crate) fn on_sim_click<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace_aether_ui_interaction(
            "transport.simulate.disabled",
            "runtime-host has no simulate action",
        );
        cx.stop_propagation();
    }
    pub(crate) fn on_step_click<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace_aether_ui_interaction(
            "transport.step.disabled",
            "runtime-host has no step-frame action",
        );
        cx.stop_propagation();
    }
    pub(crate) fn on_stop_click<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        trace_aether_ui_interaction("transport.stop", "dispatch=StopEditorWorld");
        window.dispatch_action(
            Box::new(az_editor_ui::actions::StopEditorWorld { preserve: false }),
            cx,
        );
        cx.stop_propagation();
    }
    pub(crate) fn toggle_angle_snap<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.dispatch_action(Box::new(az_editor_ui::actions::ToggleSceneAngleSnap), cx);
        cx.stop_propagation();
    }
    pub(crate) fn toggle_grid_snap<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.dispatch_action(Box::new(az_editor_ui::actions::ToggleSceneGridSnap), cx);
        cx.stop_propagation();
    }
    fn animation_asset_rows(&self, status: &EditorAssetBrowserStatus) -> Vec<AetherItem> {
        authored_asset_entries(status)
            .filter(|entry| entry_is_animation(entry))
            .take(32)
            .enumerate()
            .map(|(index, entry)| animation_asset_row(entry, index == 0))
            .collect()
    }

    fn skeleton_asset_rows(&self, status: &EditorAssetBrowserStatus) -> Vec<AetherItem> {
        let skeleton_rows = authored_asset_entries(status)
            .filter(|entry| entry_looks_like_skeleton(entry))
            .take(32)
            .enumerate()
            .map(|(index, entry)| skeleton_asset_row(entry, index == 0))
            .collect::<Vec<_>>();
        if skeleton_rows.is_empty() {
            self.animation_asset_rows(status)
        } else {
            skeleton_rows
        }
    }

    pub(crate) fn activate_animation_item_with_window(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match item.kind.as_str() {
            "anim-motion" | "anim-set-motion" | "anim-clip" => {
                let motion_glb = non_empty_string_or(&item.src, &item.key);
                if motion_glb.is_empty() {
                    return false;
                }
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAnimationMotion { motion_glb }),
                    cx,
                );
                cx.stop_propagation();
                cx.notify();
                true
            }
            "anim-blend-space" => {
                let bspace_ron_path = non_empty_string_or(&item.src, &item.key);
                if bspace_ron_path.is_empty() {
                    return false;
                }
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAnimationBlendSpace { bspace_ron_path }),
                    cx,
                );
                cx.stop_propagation();
                cx.notify();
                true
            }
            "anim-character" => {
                let character_glb = non_empty_string_or(&item.src, &item.key);
                if character_glb.is_empty() {
                    return false;
                }
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectAnimationCharacter { character_glb }),
                    cx,
                );
                cx.stop_propagation();
                cx.notify();
                true
            }
            "anim-fragment" | "anim-fragment-option" | "anim-fragment-transition" => {
                let fragment_key = non_empty_string_or(&item.src, &item.key);
                if fragment_key.is_empty() {
                    return false;
                }
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SelectMannequinFragment { fragment_key }),
                    cx,
                );
                cx.stop_propagation();
                cx.notify();
                true
            }
            "anim-transport" => {
                self.dispatch_animation_transport_action(item, window, cx);
                true
            }
            _ => false,
        }
    }

    fn dispatch_animation_transport_action(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match item.key.as_str() {
            "play-pause" => {
                let playing = !cx
                    .try_global::<EditorMannequinPreview>()
                    .is_some_and(|preview| preview.playing);
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SetAnimationPreviewPlaying { playing }),
                    cx,
                );
            }
            "stop" => {
                window.dispatch_action(Box::new(az_editor_ui::actions::StopAnimationPreview), cx);
            }
            "loop" => {
                let looping = !cx
                    .try_global::<EditorMannequinPreview>()
                    .is_some_and(|preview| preview.looping);
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::SetAnimationPreviewLoop { looping }),
                    cx,
                );
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }
    pub(crate) fn active_mode_title(&self) -> String {
        aether_mode_items()
            .into_iter()
            .find(|item| item.key == self.state.workspace_projection().mode)
            .map(|item| item.title)
            .unwrap_or_else(|| "Scene".to_owned())
    }
    pub(crate) fn active_mode_icon(&self) -> String {
        aether_mode_items()
            .into_iter()
            .find(|item| item.key == self.state.workspace_projection().mode)
            .map(|item| item.icon)
            .unwrap_or_else(|| "view_in_ar".to_owned())
    }
}

fn aether_mode_items() -> Vec<AetherItem> {
    [
        ("scene", "Scene", "view_in_ar"),
        ("sequencer", "Sequencer", "movie"),
        ("animation", "Animation Editor", "directions_run"),
        ("materials", "Material Editor", "palette"),
        ("scripting", "Script Editor", "code"),
        ("gamedata", "Game Data", "table_chart"),
    ]
    .into_iter()
    .map(|(key, title, icon)| AetherItem {
        key: key.to_owned(),
        title: title.to_owned(),
        icon: icon.to_owned(),
        ..AetherItem::default()
    })
    .collect()
}

fn scene_tool_item(
    tool: EditorSceneToolKind,
    active: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        kind: "scene-tool".to_owned(),
        key: tool.key().to_owned(),
        label: tool.label().to_owned(),
        title: tool.title().to_owned(),
        icon: tool.icon().to_owned(),
        active,
        selected: active,
        ..AetherItem::default()
    };
    set_item_style(
        &mut item,
        "style",
        scene_toolbar_button_style(active, theme),
    );
    item
}

fn scene_pivot_item(
    pivot: EditorScenePivot,
    selected: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        kind: "scene-pivot".to_owned(),
        key: pivot.key().to_owned(),
        label: pivot.label().to_owned(),
        active: selected,
        selected,
        ..AetherItem::default()
    };
    set_item_style(
        &mut item,
        "style",
        scene_segmented_option_style(selected, theme),
    );
    item
}

fn scene_space_item(
    space: EditorSceneTransformSpace,
    selected: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        kind: "scene-space".to_owned(),
        key: space.key().to_owned(),
        label: space.label().to_owned(),
        active: selected,
        selected,
        ..AetherItem::default()
    };
    set_item_style(
        &mut item,
        "style",
        scene_segmented_option_style(selected, theme),
    );
    item
}

fn scene_toolbar_button_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("width", "30px".to_owned()),
        ("height", "24px".to_owned()),
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "center".to_owned()),
        ("borderRadius", "5px".to_owned()),
        ("cursor", "pointer".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.accent
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            hsla_css(if active {
                theme.accent.opacity(0.16)
            } else {
                theme.transparent
            }),
        ),
    ])
}

fn scene_segmented_option_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "center".to_owned()),
        ("height", "22px".to_owned()),
        ("padding", "0 9px".to_owned()),
        ("borderRadius", "4px".to_owned()),
        ("fontSize", "11px".to_owned()),
        ("cursor", "pointer".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            hsla_css(if active {
                theme.secondary_active
            } else {
                theme.transparent
            }),
        ),
    ])
}

pub(crate) fn scene_snap_style(enabled: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "5px".to_owned()),
        ("height", "28px".to_owned()),
        ("padding", "0 9px".to_owned()),
        ("borderRadius", "5px".to_owned()),
        ("cursor", "pointer".to_owned()),
        (
            "color",
            hsla_css(if enabled {
                theme.accent
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            hsla_css(if enabled {
                theme.accent.opacity(0.12)
            } else {
                theme.transparent
            }),
        ),
    ])
}

pub(crate) fn mode_button_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("width", "34px".to_owned()),
        ("height", "34px".to_owned()),
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "center".to_owned()),
        ("borderRadius", "7px".to_owned()),
        ("cursor", "pointer".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            hsla_css(if active {
                theme.secondary
            } else {
                theme.transparent
            }),
        ),
    ])
}

fn authored_asset_entries(
    status: &EditorAssetBrowserStatus,
) -> impl Iterator<Item = &AssetBrowserEntryData> {
    status.entries.iter().filter(|entry| {
        entry
            .schema_type
            .as_deref()
            .is_some_and(|schema| !schema.trim().is_empty())
    })
}

fn entry_is_animation(entry: &AssetBrowserEntryData) -> bool {
    entry
        .schema_type
        .as_deref()
        .is_some_and(|schema| asset_kind(schema, schema) == EditorTypeKind::Animation)
}

fn entry_looks_like_skeleton(entry: &AssetBrowserEntryData) -> bool {
    let schema = entry
        .schema_type
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = entry.source_path.to_ascii_lowercase().replace('\\', "/");
    schema.contains("skeleton")
        || schema.contains("skel")
        || schema.contains("mannequin")
        || path.contains("/skeleton")
        || path.ends_with(".skel")
        || path.ends_with(".skeleton")
        || path.ends_with(".chr")
        || path.ends_with(".cdf")
}

fn source_file_name(source_path: &str) -> String {
    source_path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(source_path)
        .to_owned()
}

fn animation_entry_activity_label(entry: &AssetBrowserEntryData) -> String {
    if entry.diagnostics_count > 0 {
        return format!("{} issues", entry.diagnostics_count);
    }
    match entry.latest_job.as_ref().map(|job| job.status) {
        Some(AssetBrowserJobStatus::Queued) => "queued".to_owned(),
        Some(AssetBrowserJobStatus::Leased) => "leased".to_owned(),
        Some(AssetBrowserJobStatus::Succeeded) => "built".to_owned(),
        Some(AssetBrowserJobStatus::Failed) => "failed".to_owned(),
        Some(AssetBrowserJobStatus::Abandoned) => "abandoned".to_owned(),
        None => "indexed".to_owned(),
    }
}

fn animation_asset_row(entry: &AssetBrowserEntryData, active: bool) -> AetherItem {
    let mut item = AetherItem {
        key: entry.entry_id.to_string(),
        name: path_stem_label(&entry.source_path),
        file: source_file_name(&entry.source_path),
        frames: animation_entry_activity_label(entry),
        loop_icon: "repeat".to_owned(),
        active,
        selected: active,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", animation_asset_row_style(active));
    set_item_style(&mut item, "nameStyle", hierarchy_name_style(active));
    set_item_style(&mut item, "hitStyle", blend_sample_hit_style(active, 12.0));
    set_item_style(&mut item, "dotStyle", blend_sample_dot_style(active));
    set_item_style(
        &mut item,
        "lblStyle",
        blend_sample_label_style(active, 12.0),
    );
    item.label = item.name.clone();
    item
}

fn skeleton_asset_row(entry: &AssetBrowserEntryData, active: bool) -> AetherItem {
    let mut item = animation_asset_row(entry, active);
    item.icon = "accessibility_new".to_owned();
    item.file = source_file_name(&entry.source_path);
    item.frames = entry
        .schema_type
        .clone()
        .unwrap_or_else(|| "skeleton".to_owned());
    item
}

fn material_slider_fill_style(value: f32) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "absolute".to_owned()),
        ("left", "0".to_owned()),
        ("top", "0".to_owned()),
        ("height", "100%".to_owned()),
        ("width", format!("{:.0}%", value * 100.0)),
        ("background", "#4188e0".to_owned()),
        ("borderRadius", "2px".to_owned()),
    ])
}

fn animation_asset_row_style(active: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "9px".to_owned()),
        ("height", "29px".to_owned()),
        ("padding", "0 10px".to_owned()),
        ("cursor", "default".to_owned()),
        (
            "background",
            if active {
                "rgba(65,136,224,0.13)"
            } else {
                "transparent"
            }
            .to_owned(),
        ),
        (
            "borderLeft",
            if active {
                "2px solid #4188e0"
            } else {
                "2px solid transparent"
            }
            .to_owned(),
        ),
    ])
}

fn blend_sample_hit_style(active: bool, x: f32) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "absolute".to_owned()),
        ("left", format!("{x:.1}%")),
        ("top", "50%".to_owned()),
        ("cursor", "grab".to_owned()),
        ("opacity", if active { "1" } else { "0.75" }.to_owned()),
    ])
}

fn blend_sample_dot_style(active: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("width", "10px".to_owned()),
        ("height", "10px".to_owned()),
        (
            "background",
            if active { "#4188e0" } else { "#7a8aa0" }.to_owned(),
        ),
        ("border", "1px solid #15171b".to_owned()),
    ])
}

fn blend_sample_label_style(active: bool, x: f32) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "absolute".to_owned()),
        ("left", format!("{x:.1}%")),
        ("top", "calc(50% + 12px)".to_owned()),
        ("transform", "translateX(-50%)".to_owned()),
        ("fontSize", "10px".to_owned()),
        (
            "color",
            if active { "#dce5f0" } else { "#8a99ac" }.to_owned(),
        ),
    ])
}
