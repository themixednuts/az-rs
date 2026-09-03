//! Workspace shell projection and cross-region interaction routing.

use std::path::Path;

use az_editor_ui::panels::project_workflow;
use az_editor_ui::{
    EditorScenePivot, EditorSceneToolKind, EditorSceneToolState, EditorSceneTransformSpace,
};
use gpui::{AppContext, Context, Pixels, Point, Rgba, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherItems, AetherStyle};
use crate::app::aether_editor_model::{
    AetherConsoleFilter, AetherEditorState, OverlayDismissal, trace_aether_ui_render_state,
    trace_aether_ui_state, trace_value,
};
use crate::app::aether_editor_view::AetherEditorView;
use crate::attach::EditorAttachSession;
use crate::game_data_catalog::EditorGameDataCatalog;

use super::super::AetherViewAction;
use super::super::authored_content::{mode_button_style, scene_snap_style};
use super::super::gamedata_gem::game_data_table_for_item;
use super::super::presentation::{
    aether_tab_item, bottom_tab_badge_style, hsla_css, item_can_expand, set_item_style,
    toolbar_segment_style,
};
use super::dock::panel_button_style;
use super::menus_settings::{menu_button_style, select_popover_style, source_control_projection};

impl AetherEditorView {
    pub(crate) fn actions(&self) -> Vec<AetherItem> {
        Vec::new()
    }
    pub(crate) fn layout_items(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        aether_layout_items(&theme)
    }
    pub(crate) fn left_tabs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut tabs = aether_left_tabs();
        self.apply_collection_state("leftTabs", &mut tabs, &theme);
        tabs
    }
    pub(crate) fn right_tabs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut tabs = aether_right_tabs();
        self.apply_collection_state("rightTabs", &mut tabs, &theme);
        tabs.into_iter()
            .filter(|item| item.key != "prefab" || self.is_selected_prefab_instance(cx))
            .collect()
    }
    pub(crate) fn right_toggles(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut toggles = aether_right_toggles();
        self.apply_collection_state("rightToggles", &mut toggles, &theme);
        toggles
    }
    pub(crate) fn view_pills(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut pills = aether_view_pills(self.state.diagnostics_presentation().show_stats, &theme);
        self.apply_collection_state("viewPills", &mut pills, &theme);
        pills
    }
    pub(crate) fn view_tabs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut tabs = aether_view_tabs();
        self.apply_collection_state("viewTabs", &mut tabs, &theme);
        tabs
    }
    pub(crate) fn left_toggle(&self, cx: &mut Context<Self>) -> AetherItem {
        let theme = cx.theme().clone();
        let enabled = self.state.panel_capabilities().left;
        let mut item = AetherItem {
            icon: "dock_to_left".to_owned(),
            title: "Toggle left panel (Hierarchy)".to_owned(),
            ..AetherItem::default()
        };
        set_item_style(
            &mut item,
            "style",
            panel_button_style(self.state.workspace_projection().show_left, enabled, &theme),
        );
        item
    }
    pub(crate) fn style(&self) -> AetherItem {
        AetherItem::default()
    }
    pub(crate) fn scm_seg_style(&self) -> AetherStyle {
        toolbar_segment_style(true, 6)
    }
    pub(crate) fn sel_seg_style(&self) -> AetherStyle {
        toolbar_segment_style(true, 6)
    }
    pub(crate) fn diag_seg_style(&self) -> AetherStyle {
        toolbar_segment_style(true, 9)
    }
    pub(crate) fn is_about(&self) -> bool {
        {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "about"
        }
    }
    pub(crate) fn is_settings(&self) -> bool {
        {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "preferences"
        }
    }
    pub(crate) fn layout_open(&self) -> bool {
        self.bool_value("layoutOpen")
    }
    pub(crate) fn left_hierarchy(&self) -> bool {
        self.bool_value("leftHierarchy")
    }
    pub(crate) fn left_layers(&self) -> bool {
        self.bool_value("leftLayers")
    }
    pub(crate) fn menu_open(&self) -> bool {
        self.bool_value("menuOpen")
    }
    pub(crate) fn modal_open(&self) -> bool {
        self.bool_value("modalOpen")
    }
    pub(crate) fn open(&self) -> bool {
        self.bool_value("buildOpen")
    }
    pub(crate) fn select_popover_open(&self) -> bool {
        self.bool_value("selectPopoverOpen")
    }
    pub(crate) fn show_stats(&self) -> bool {
        self.bool_value("showStats")
    }
    pub(crate) fn tab_assets(&self) -> bool {
        self.bool_value("tabAssets")
    }
    pub(crate) fn view_menu_open(&self) -> bool {
        self.state.view_pill_menu_open()
    }
    pub(crate) fn project_file_title(&self, cx: &mut Context<Self>) -> String {
        if let Some(session) = cx.try_global::<EditorAttachSession>() {
            let project_name = session
                .project_root
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&session.project_id);
            return format!("{project_name}.aeproj");
        }
        if let Some(status) = cx.try_global::<project_workflow::Status>()
            && let Some(project_root) = status.project_root.as_deref()
            && let Some(project_name) = Path::new(project_root)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.trim().is_empty())
        {
            return format!("{project_name}.aeproj");
        }
        "No Project".to_owned()
    }
    pub(crate) fn editor_version_label(&self) -> String {
        format!("Aether {}", env!("CARGO_PKG_VERSION"))
    }
    pub(crate) fn about_engine_name(&self) -> &'static str {
        "Azoth Engine"
    }
    pub(crate) fn about_version_label(&self) -> String {
        format!("{} {}", env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION"))
    }
    pub(crate) fn cur_x(&self) -> String {
        self.string_value("curX")
    }
    pub(crate) fn cur_y(&self) -> String {
        self.string_value("curY")
    }
    pub(crate) fn cur_z(&self) -> String {
        self.string_value("curZ")
    }
    pub(crate) fn gizmo_mode_label(&self, cx: &mut Context<Self>) -> String {
        let state = cx
            .try_global::<EditorSceneToolState>()
            .cloned()
            .unwrap_or_default();
        format!(
            "{} · {} · {}",
            state.tool.label(),
            state.pivot.label(),
            state.space.label()
        )
    }
    pub(crate) fn icon(&self) -> String {
        self.string_value("modeIcon")
    }
    pub(crate) fn layout(&self) -> String {
        self.string_value("layout")
    }
    pub(crate) fn layout_icon(&self) -> String {
        self.string_value("layoutIcon")
    }
    pub(crate) fn modal_icon(&self) -> String {
        match self.state.overlay_projection().modal_kind.as_str() {
            "about" => "info".to_owned(),
            "preferences" => "tune".to_owned(),
            _ => self.string_value("modalIcon"),
        }
    }
    pub(crate) fn modal_subtitle(&self) -> String {
        match self.state.overlay_projection().modal_kind.as_str() {
            "about" => "Engine and renderer facts".to_owned(),
            "preferences" => "Editor settings saved to the global Azoth settings store".to_owned(),
            _ => self.string_value("modalSubtitle"),
        }
    }
    pub(crate) fn modal_title(&self) -> String {
        match self.state.overlay_projection().modal_kind.as_str() {
            "about" => "About Azoth".to_owned(),
            "preferences" => "Preferences".to_owned(),
            _ => self.string_value("modalTitle"),
        }
    }
    pub(crate) fn scm_branch(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorAttachSession>()
            .and_then(source_control_projection)
            .map(|segment| segment.branch)
            .unwrap_or_default()
    }
    pub(crate) fn scm_dirty(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorAttachSession>()
            .and_then(source_control_projection)
            .map(|segment| segment.change_count.to_string())
            .unwrap_or_default()
    }
    pub(crate) fn sub_loaded_label(&self) -> String {
        self.string_value("subLoadedLabel")
    }
    pub(crate) fn title(&self) -> String {
        self.string_value("modeTitle")
    }
    pub(crate) fn close_layout_menu<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::CloseLayoutMenu);
        cx.notify();
    }
    pub(crate) fn close_level_menu<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::CloseLevelMenu);
        cx.notify();
    }
    pub(crate) fn close_menu<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::CloseBuildAndMenu);
        cx.notify();
    }
    pub(crate) fn close_modal<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissal = if {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "preferences"
        } {
            self.state
                .cancel_preferences_modal_state()
                .map_or(OverlayDismissal::Closed, OverlayDismissal::ApplySettings)
        } else {
            self.state.close_modal_state();
            OverlayDismissal::Closed
        };
        self.apply_overlay_dismissal(dismissal, "modal-cancel", window, cx);
        cx.notify();
    }
    pub(crate) fn confirm_modal<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissal = if {
            let overlay = self.state.overlay_projection();
            overlay.modal_open && overlay.modal_kind == "preferences"
        } {
            self.state
                .confirm_preferences_modal_state()
                .map_or(OverlayDismissal::Closed, OverlayDismissal::PersistSettings)
        } else {
            self.state.close_modal_state();
            OverlayDismissal::Closed
        };
        self.apply_overlay_dismissal(dismissal, "modal-confirm", window, cx);
        cx.notify();
    }
    pub(crate) fn activate_item(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if item.kind == "menu-action" {
            self.activate_menu_item(item, window, cx);
            return;
        }
        if self.activate_animation_item_with_window(item, window, cx) {
            return;
        }
        if self.activate_gamedata_item_with_window(item, window, cx) {
            return;
        }
        let before = self.state.trace_summary();
        let item_detail = format!(
            "key={} name={} label={} title={}",
            trace_value(&item.key),
            trace_value(&item.name),
            trace_value(&item.label),
            trace_value(&item.title)
        );
        if item.sep {
            trace_aether_ui_state(
                "item.activate_ignored",
                format!("{item_detail} reason=separator before={before}"),
                &self.state,
            );
            return;
        }

        if matches!(item.id.as_str(), "gamedata-table" | "gamedata-table-link") {
            if let Some(table) = cx
                .try_global::<EditorGameDataCatalog>()
                .and_then(|catalog| game_data_table_for_item(&catalog.catalog, item))
                .cloned()
            {
                self.state.select_gamedata_table_state(&table);
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=gamedata_table_state before={before}"),
                    &self.state,
                );
                cx.notify();
                return;
            }
        }

        if item.id == "gamedata-schema" {
            self.state.select_gamedata_schema_state(&item.key);
            trace_aether_ui_state(
                "item.activate",
                format!("{item_detail} route=gamedata_schema before={before}"),
                &self.state,
            );
            cx.notify();
            return;
        }

        match item.kind.as_str() {
            "gem" => {
                self.state.select_gem_state(&item.key);
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=gem_select before={before}"),
                    &self.state,
                );
                cx.notify();
                return;
            }
            "layer" => {
                self.state.select_left_tab_state("layers");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=layer_select before={before}"),
                    &self.state,
                );
                cx.notify();
                return;
            }
            "level" | "level-action" => {
                self.state.close_level_state();
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route={} before={before}", item.kind),
                    &self.state,
                );
                cx.notify();
                return;
            }
            _ => {}
        }

        let key = item.key.as_str();
        if !key.is_empty() {
            if let Some(filter) = AetherConsoleFilter::from_key(key) {
                self.state.set_console_filter_state(filter);
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=console_filter before={before}"),
                    &self.state,
                );
                cx.notify();
                return;
            }
            match key {
                "stats" => {
                    self.state.toggle_stats_state();
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=toggle_stats before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                "scene" | "sequencer" | "animation" | "materials" | "scripting" | "gamedata" => {
                    crate::perf::begin_interaction(crate::perf::ACTIVITY_MODE_TO_WORKSPACE);
                    self.set_mode_with_window(key, window, cx);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=mode_layout before={before}"),
                        &self.state,
                    );
                    return;
                }
                "hierarchy" | "layers" => {
                    self.state.select_left_tab_state(key);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=left_tab before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                "details" | "prefab" => {
                    self.state.select_right_tab_state(key);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=right_tab before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                "perspective" | "game" => {
                    self.state.select_view_tab_state(key);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=view_tab before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                "assets" | "console" | "output" | "profiler" | "gems" => {
                    self.state.select_bottom_tab_state(key);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=bottom_tab before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                "tables" | "schemas" | "managers" => {
                    self.state.set_game_data_view_state(key);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=game_data_view before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                "table" | "field" | "schema" | "manager" => {
                    self.state.set_game_data_tab_state(key);
                    trace_aether_ui_state(
                        "item.activate",
                        format!("{item_detail} route=game_data_tab before={before}"),
                        &self.state,
                    );
                    cx.notify();
                    return;
                }
                _ => {}
            }
        }

        match item.label.as_str() {
            "Asset Browser" => {
                self.state.select_bottom_tab_state("assets");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=bottom_tab_label before={before}"),
                    &self.state,
                );
            }
            "Console" => {
                self.state.select_bottom_tab_state("console");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=bottom_tab_label before={before}"),
                    &self.state,
                );
            }
            "Output Log" => {
                self.state.select_bottom_tab_state("output");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=bottom_tab_label before={before}"),
                    &self.state,
                );
            }
            "Profiler" => {
                self.state.select_bottom_tab_state("profiler");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=bottom_tab_label before={before}"),
                    &self.state,
                );
            }
            "Gems" => {
                self.state.select_bottom_tab_state("gems");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=bottom_tab_label before={before}"),
                    &self.state,
                );
            }
            "Tables" => {
                self.state.set_game_data_view_state("tables");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=game_data_view_label before={before}"),
                    &self.state,
                );
            }
            "Schemas" => {
                self.state.set_game_data_view_state("schemas");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=game_data_view_label before={before}"),
                    &self.state,
                );
            }
            "Managers" => {
                self.state.set_game_data_view_state("managers");
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=game_data_view_label before={before}"),
                    &self.state,
                );
            }
            _ if self.toggle_expandable_item_state(item, false) => trace_aether_ui_state(
                "item.activate",
                format!("{item_detail} route=toggle_expandable before={before}"),
                &self.state,
            ),
            _ => {
                self.state.close_menu_state();
                trace_aether_ui_state(
                    "item.activate",
                    format!("{item_detail} route=close_menu_fallback before={before}"),
                    &self.state,
                );
            }
        }
        cx.notify();
    }

    pub(crate) fn activate_toolbar_item(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::perf::begin_interaction(crate::perf::TOOLBAR_TO_VISIBLE);
        match item.kind.as_str() {
            "scene-tool" => {
                if let Some(tool) = EditorSceneToolKind::from_key(&item.key) {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::SetSceneTool { tool }),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
            }
            "scene-pivot" => {
                if let Some(pivot) = EditorScenePivot::from_key(&item.key) {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::SetScenePivot { pivot }),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
            }
            "scene-space" => {
                if let Some(space) = EditorSceneTransformSpace::from_key(&item.key) {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::SetSceneTransformSpace { space }),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
            }
            "graph-auto-layout" => {
                window.dispatch_action(Box::new(az_editor_ui::actions::AutoLayoutGraph), cx);
                cx.stop_propagation();
                return;
            }
            "graph-create-comment" => {
                window.dispatch_action(
                    Box::new(az_editor_ui::actions::CreateGraphComment {
                        text: "Comment".to_owned(),
                        x: 40.0,
                        y: 40.0,
                        width: 220.0,
                        height: 96.0,
                    }),
                    cx,
                );
                cx.stop_propagation();
                return;
            }
            "gamedata-validate" => {
                window.dispatch_action(Box::new(az_editor_ui::actions::RefreshGameDataCatalog), cx);
                cx.stop_propagation();
                return;
            }
            "build-profile" => {
                if !item.key.is_empty() {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::SetProjectBuildProfile {
                            profile: item.key.clone(),
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
            }
            "build-target" => {
                if !item.key.is_empty() {
                    window.dispatch_action(
                        Box::new(az_editor_ui::actions::SetProjectBuildTarget {
                            target_key: item.key.clone(),
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                    return;
                }
            }
            "build-action" if item.key == "execute-build" => {
                self.state.close_build_state();
                window.dispatch_action(Box::new(az_editor_ui::actions::ExecuteProjectBuild), cx);
                cx.stop_propagation();
                cx.notify();
                return;
            }
            "build-action" if item.key == "plan-build" => {
                self.state.close_build_state();
                window.dispatch_action(Box::new(az_editor_ui::actions::PlanProjectBuild), cx);
                cx.stop_propagation();
                cx.notify();
                return;
            }
            _ => {}
        }

        self.activate_item(item, window, cx);
    }

    fn item(&self, _key: &str) -> AetherItem {
        AetherItem::default()
    }

    pub(crate) fn style_value(&self, key: &str) -> AetherStyle {
        match key {
            _ => AetherStyle::default(),
        }
    }

    pub(crate) fn themed_style_value(&self, key: &str, cx: &mut Context<Self>) -> AetherStyle {
        let theme = cx.theme().clone();
        match key {
            _ => self.style_value(key),
        }
    }

    pub(crate) fn select_popover_style(&self, cx: &mut Context<Self>) -> AetherStyle {
        select_popover_style(&cx.theme().clone(), self.state.add_component_select_open())
    }

    pub(crate) fn grid_snap_style(&self, cx: &mut Context<Self>) -> AetherStyle {
        let enabled = cx
            .try_global::<EditorSceneToolState>()
            .is_none_or(|state| state.grid_snap.enabled);
        scene_snap_style(enabled, &cx.theme().clone())
    }

    pub(crate) fn angle_snap_style(&self, cx: &mut Context<Self>) -> AetherStyle {
        let enabled = cx
            .try_global::<EditorSceneToolState>()
            .is_none_or(|state| state.angle_snap.enabled);
        scene_snap_style(enabled, &cx.theme().clone())
    }

    pub(crate) fn string_value(&self, key: &str) -> String {
        match key {
            "modeTitle" => self.active_mode_title(),
            "modeIcon" => self.active_mode_icon(),
            _ => String::new(),
        }
    }

    pub(crate) fn bool_value(&self, key: &str) -> bool {
        let workspace = self.state.workspace_projection();
        let overlay = self.state.overlay_projection();
        match key {
            "modeScene" => workspace.mode == "scene",
            "modeSequencer" => workspace.mode == "sequencer",
            "modeAnimation" => workspace.mode == "animation",
            "modeMaterials" => workspace.mode == "materials",
            "modeScripting" => workspace.mode == "scripting",
            "modeData" => workspace.mode == "gamedata",
            "tabAssets" => workspace.bottom_tab == "assets",
            "tabConsole" => workspace.bottom_tab == "console",
            "tabOutput" => workspace.bottom_tab == "output",
            "tabProfiler" => workspace.bottom_tab == "profiler",
            "tabGems" => workspace.bottom_tab == "gems",
            "assetGrid" => self.state.asset_browser_navigation().grid,
            "assetList" => !self.state.asset_browser_navigation().grid,
            "buildOpen" => overlay.build_open,
            "menuOpen" => overlay.menu_open,
            "levelOpen" => overlay.level_open,
            "layoutOpen" => overlay.layout_open,
            "pipeOpen" => overlay.pipe_open,
            "modalOpen" => overlay.modal_open,
            "selectPopoverOpen" => overlay.select_popover_open,
            "showStats" => self.state.diagnostics_presentation().show_stats,
            "showLeft" => workspace.show_left,
            "showRight" => workspace.show_right,
            "showBottom" => workspace.show_bottom,
            _ => false,
        }
    }

    pub(crate) fn apply_collection_state(
        &self,
        key: &str,
        items: &mut [AetherItem],
        theme: &gpui_component::theme::Theme,
    ) {
        let workspace = self.state.workspace_projection();
        let overlay = self.state.overlay_projection();
        let diagnostics = self.state.diagnostics_presentation();
        match key {
            "modes" => {
                trace_aether_ui_render_state("collection.modes", "render modes", &self.state);
                for item in &mut *items {
                    item.active = item.key == workspace.mode;
                    set_item_style(item, "style", mode_button_style(item.active, theme));
                }
            }
            "leftTabs" => {
                trace_aether_ui_render_state(
                    "collection.left_tabs",
                    "render leftTabs",
                    &self.state,
                );
                mark_active(items, &workspace.left_tab);
                refresh_tab_styles(items, theme);
            }
            "rightTabs" => {
                trace_aether_ui_render_state(
                    "collection.right_tabs",
                    "render rightTabs",
                    &self.state,
                );
                mark_active(items, &workspace.right_tab);
                refresh_tab_styles(items, theme);
            }
            "viewTabs" => {
                trace_aether_ui_render_state(
                    "collection.view_tabs",
                    "render viewTabs",
                    &self.state,
                );
                mark_active(items, &workspace.view_tab);
                refresh_tab_styles(items, theme);
            }
            "viewPills" => {
                trace_aether_ui_render_state(
                    "collection.view_pills",
                    "render viewPills",
                    &self.state,
                );
                for item in &mut *items {
                    if item.key != "show" {
                        continue;
                    }
                    for child in &mut item.items.0 {
                        if child.key == "stats" {
                            child.check_icon = if diagnostics.show_stats {
                                "check_box"
                            } else {
                                "check_box_outline_blank"
                            }
                            .to_owned();
                            child.check_color = hsla_css(if diagnostics.show_stats {
                                theme.accent
                            } else {
                                theme.muted_foreground
                            });
                            set_item_style(
                                child,
                                "style",
                                view_pill_item_style(diagnostics.show_stats, theme),
                            );
                        }
                    }
                }
            }
            "bottomTabs" => {
                trace_aether_ui_render_state(
                    "collection.bottom_tabs",
                    "render bottomTabs",
                    &self.state,
                );
                mark_active(items, &workspace.bottom_tab);
                refresh_tab_styles(items, theme);
            }
            "menus" => {
                trace_aether_ui_render_state("collection.menus", "render menus", &self.state);
                for item in &mut *items {
                    item.open = overlay.menu_open && item.name == overlay.open_menu;
                    set_item_style(item, "btnStyle", menu_button_style(item.open, theme));
                }
            }
            "rightToggles" => {
                trace_aether_ui_render_state(
                    "collection.right_toggles",
                    "render rightToggles",
                    &self.state,
                );
                let capabilities = self.state.panel_capabilities();
                for item in &mut *items {
                    let active = match item.key.as_str() {
                        "l" => workspace.show_left,
                        "b" => workspace.show_bottom,
                        "r" => workspace.show_right,
                        _ => false,
                    };
                    let enabled = match item.key.as_str() {
                        "l" => capabilities.left,
                        "b" => capabilities.bottom,
                        "r" => capabilities.right,
                        _ => false,
                    };
                    set_item_style(item, "style", panel_button_style(active, enabled, theme));
                }
            }
            _ => {}
        }
        apply_item_expansion_overrides(&self.state, items);
    }

    pub(crate) fn apply_input(&mut self, name: &str, value: &str) {
        match name {
            "asset_on_search" => {
                self.state.search_assets_state(value);
            }
            "asset_create_on_name" => {
                self.state.edit_asset_create_name_state(value);
            }
            "asset_create_on_folder" => {
                self.state.edit_asset_create_folder_state(value);
            }
            "asset_rename_on_path" => {
                self.state.edit_asset_rename_target_state(value);
            }
            "console_on_filter" => {
                self.state.set_console_query_state(value);
            }
            _ => {}
        }
    }

    pub(super) fn open_preferences_modal(&mut self, cx: &mut Context<Self>) {
        let settings = self.editor_settings(cx);
        self.state.open_preferences_modal(settings);
    }

    pub(super) fn open_about_modal(&mut self) {
        self.state.open_about_modal_state();
    }

    pub(in crate::app) fn apply_action(&mut self, action: AetherViewAction) {
        let before = self.state.trace_summary();
        match action {
            AetherViewAction::GoAssets => {
                self.state.select_bottom_tab_state("assets");
            }
            AetherViewAction::GoConsole => {
                self.state.select_bottom_tab_state("console");
            }
            AetherViewAction::GoOutput => {
                self.state.select_bottom_tab_state("output");
                self.state.close_pipe_state();
            }
            AetherViewAction::GoProfiler => {
                self.state.select_bottom_tab_state("profiler");
            }
            AetherViewAction::UseAssetGrid => {
                self.state.choose_asset_grid_layout_state(true);
            }
            AetherViewAction::UseAssetList => {
                self.state.choose_asset_grid_layout_state(false);
            }
            AetherViewAction::TogglePipe => {
                self.state.toggle_pipe_state();
            }
            AetherViewAction::ClosePipe => self.state.close_pipe_state(),
            AetherViewAction::ToggleBuild => {
                self.state.toggle_build_state();
            }
            AetherViewAction::CloseBuild => self.state.close_build_state(),
            AetherViewAction::CloseBuildAndMenu => self.state.close_build_and_menu_state(),
            AetherViewAction::ToggleLevelMenu => {
                self.state.toggle_level_state();
            }
            AetherViewAction::CloseLevelMenu => self.state.close_level_state(),
            AetherViewAction::ToggleLayoutMenu => {
                self.state.toggle_layout_state();
            }
            AetherViewAction::CloseLayoutMenu => self.state.close_layout_state(),
        }
        let after = self.state.trace_summary();
        let event = if before == after {
            "action.noop"
        } else {
            "action.apply"
        };
        trace_aether_ui_state(
            event,
            format!("action={action:?} before={before}"),
            &self.state,
        );
    }
}

fn mark_active(items: &mut [AetherItem], key: &str) {
    for item in items {
        item.active = item.key == key;
        item.selected = item.key == key;
    }
}

fn refresh_tab_styles(items: &mut [AetherItem], theme: &gpui_component::theme::Theme) {
    for item in items {
        set_item_style(item, "style", tab_style(item.active, theme));
    }
}

fn aether_left_tabs() -> Vec<AetherItem> {
    [
        ("hierarchy", "Hierarchy", "account_tree"),
        ("layers", "Layers", "layers"),
    ]
    .into_iter()
    .map(aether_tab_item)
    .collect()
}

fn aether_right_tabs() -> Vec<AetherItem> {
    [
        ("details", "Details", "tune"),
        ("prefab", "Prefab", "widgets"),
    ]
    .into_iter()
    .map(aether_tab_item)
    .collect()
}

fn aether_view_tabs() -> Vec<AetherItem> {
    [
        ("persp", "Perspective", "view_in_ar"),
        ("game", "Game", "sports_esports"),
    ]
    .into_iter()
    .map(aether_tab_item)
    .collect()
}

fn aether_right_toggles() -> Vec<AetherItem> {
    [
        ("b", "dock_to_bottom", "Toggle bottom panel"),
        ("r", "dock_to_right", "Toggle right panel (Inspector)"),
    ]
    .into_iter()
    .map(|(key, icon, title)| AetherItem {
        key: key.to_owned(),
        icon: icon.to_owned(),
        title: title.to_owned(),
        ..AetherItem::default()
    })
    .collect()
}

fn aether_layout_items(theme: &gpui_component::theme::Theme) -> Vec<AetherItem> {
    [
        ("Default", "Default", "dashboard", true),
        ("Animation", "Animation", "movie", false),
        ("Scripting", "Scripting", "code", false),
    ]
    .into_iter()
    .map(|(key, name, icon, active)| {
        let mut item = AetherItem {
            key: key.to_owned(),
            name: name.to_owned(),
            icon: icon.to_owned(),
            active,
            selected: active,
            ..AetherItem::default()
        };
        set_item_style(&mut item, "style", layout_item_style(active, theme));
        item
    })
    .collect()
}

fn aether_view_pills(show_stats: bool, theme: &gpui_component::theme::Theme) -> Vec<AetherItem> {
    vec![
        view_pill(
            "cam",
            "videocam",
            "Perspective",
            false,
            vec![
                view_choice("Perspective", "Perspective", "", true, theme),
                view_choice("Top", "Top", "", false, theme),
                view_choice("Front", "Front", "", false, theme),
                view_choice("Side", "Side", "", false, theme),
                view_choice("Game", "Game", "", false, theme),
            ],
            theme,
        ),
        view_pill(
            "shade",
            "wb_sunny",
            "Lit",
            false,
            vec![
                view_choice("Lit", "Lit", "", true, theme),
                view_choice("Unlit", "Unlit", "", false, theme),
                view_choice("Wireframe", "Wireframe", "", false, theme),
                view_choice("Normals", "Normals", "", false, theme),
            ],
            theme,
        ),
        view_pill(
            "show",
            "visibility",
            "Show",
            true,
            vec![
                view_check("grid", "Grid", "grid_on", true, theme),
                view_check("stats", "Stats Overlay", "speed", show_stats, theme),
                view_check("bounds", "Bounds", "crop_free", false, theme),
                view_check("skybox", "Skybox", "cloud", true, theme),
            ],
            theme,
        ),
        view_pill(
            "giz",
            "category",
            "Gizmos",
            true,
            vec![
                view_check("lights", "Light Icons", "lightbulb", true, theme),
                view_check("cameras", "Camera Icons", "videocam", true, theme),
                view_check("colliders", "Colliders", "select_all", false, theme),
                view_check("bounds", "Bounding Boxes", "crop_free", false, theme),
            ],
            theme,
        ),
    ]
}

fn view_pill(
    key: &str,
    icon: &str,
    label: &str,
    multi: bool,
    items: Vec<AetherItem>,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        key: key.to_owned(),
        icon: icon.to_owned(),
        label: label.to_owned(),
        single: !multi,
        multi,
        items: AetherItems(items),
        ..AetherItem::default()
    };
    set_item_style(&mut item, "pillStyle", view_pill_style(theme));
    item
}

fn view_choice(
    key: &str,
    label: &str,
    icon: &str,
    active: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        key: key.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        active,
        selected: active,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", view_pill_item_style(active, theme));
    item
}

fn view_check(
    key: &str,
    label: &str,
    icon: &str,
    checked: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = view_choice(key, label, icon, checked, theme);
    item.check_icon = if checked {
        "check_box"
    } else {
        "check_box_outline_blank"
    }
    .to_owned();
    item.check_color = hsla_css(if checked {
        theme.accent
    } else {
        theme.muted_foreground
    });
    item
}

fn compact_tab_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "4px".to_owned()),
        ("height", "32px".to_owned()),
        ("padding", "0 8px".to_owned()),
        ("fontSize", "10.5px".to_owned()),
        ("cursor", "pointer".to_owned()),
        ("whiteSpace", "nowrap".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.tab_active_foreground
            } else {
                theme.tab_foreground
            }),
        ),
        (
            "background",
            hsla_css(if active {
                theme.tab_active
            } else {
                theme.transparent
            }),
        ),
        (
            "borderBottom",
            if active {
                format!("2px solid {}", hsla_css(theme.list_active_border))
            } else {
                "2px solid transparent".to_owned()
            },
        ),
    ])
}

fn layout_item_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "9px".to_owned()),
        ("height", "30px".to_owned()),
        ("padding", "0 8px".to_owned()),
        ("borderRadius", "5px".to_owned()),
        ("cursor", "pointer".to_owned()),
        ("fontSize", "11.5px".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
    ])
}

fn view_pill_style(theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "relative".to_owned()),
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "5px".to_owned()),
        ("height", "24px".to_owned()),
        ("padding", "0 9px".to_owned()),
        ("background", hsla_css(theme.secondary)),
        ("border", format!("1px solid {}", hsla_css(theme.border))),
        ("borderRadius", "5px".to_owned()),
        ("color", hsla_css(theme.secondary_foreground)),
        ("fontSize", "11px".to_owned()),
        ("cursor", "pointer".to_owned()),
    ])
}

fn view_pill_item_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "9px".to_owned()),
        ("height", "28px".to_owned()),
        ("padding", "0 10px".to_owned()),
        ("cursor", "pointer".to_owned()),
        ("fontSize", "11.5px".to_owned()),
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
                theme.list_active
            } else {
                theme.transparent
            }),
        ),
    ])
}

fn tab_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "5px".to_owned()),
        ("height", "32px".to_owned()),
        ("padding", "0 10px".to_owned()),
        ("fontSize", "11px".to_owned()),
        ("cursor", "pointer".to_owned()),
        ("whiteSpace", "nowrap".to_owned()),
        ("borderRadius", "0".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.tab_active_foreground
            } else {
                theme.tab_foreground
            }),
        ),
        (
            "background",
            hsla_css(if active {
                theme.tab_active
            } else {
                theme.transparent
            }),
        ),
        (
            "borderBottom",
            if active {
                format!("2px solid {}", hsla_css(theme.list_active_border))
            } else {
                "2px solid transparent".to_owned()
            },
        ),
        ("fontWeight", if active { "500" } else { "400" }.to_owned()),
    ])
}

fn apply_item_expansion_overrides(state: &AetherEditorState, items: &mut [AetherItem]) {
    for item in items {
        if item_can_expand(item, false) {
            let default_open =
                item.open || item.caret == "arrow_drop_down" || item.caret == "expand_more";
            let open = state.item_expanded(&item.key, default_open);
            apply_expandable_item_state(item, open);
        }
    }
}

pub(super) fn apply_expandable_item_state(item: &mut AetherItem, open: bool) {
    item.open = open;
    if item.has_children || !item.caret.is_empty() {
        item.caret = if open {
            if item.caret == "expand_more" || item.caret == "chevron_right" {
                "expand_more"
            } else {
                "arrow_drop_down"
            }
        } else if item.caret == "expand_more" || item.caret == "chevron_right" {
            "chevron_right"
        } else {
            "arrow_right"
        }
        .to_owned();
    }
}

fn format_millis(millis: u32) -> String {
    let seconds = millis as f32 / 1000.0;
    format!("{seconds:.2}s")
}
