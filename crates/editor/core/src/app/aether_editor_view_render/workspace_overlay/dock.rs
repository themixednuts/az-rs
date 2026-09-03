//! Persisted dock layout and geometry controls.

use gpui::{Context, Pixels, Point, Window};

use crate::app::aether_common::AetherStyle;
use crate::app::aether_editor_model::trace_aether_ui_state;
use crate::app::aether_editor_view::AetherEditorView;

use super::super::presentation::hsla_css;

impl AetherEditorView {
    pub(crate) fn persist_workspace_state(&mut self, immediate: bool, cx: &mut Context<Self>) {
        let layout = crate::workspace::dock::capture_workspace_layout(&self.dock_area, cx);
        let (asset_view_mode, asset_folder_key) =
            crate::workspace::dock::capture_cached_asset_browser(cx).unwrap_or_else(|| {
                let project = crate::ui_state_persistence::initial_project_state(cx);
                (project.asset_view_mode, project.asset_folder_key)
            });
        crate::ui_state_persistence::update_workspace_state(
            &self.state.workspace_projection().mode,
            layout,
            &asset_view_mode,
            asset_folder_key,
            immediate,
            cx,
        );
    }

    pub(crate) fn reset_layout_action(
        &mut self,
        _action: &crate::actions::ResetLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mode = self.state.workspace_projection().mode;
        crate::ui_state_persistence::reset_layout(&mode, cx);
        crate::workspace::dock::apply_workspace_layout_for_mode(&self.dock_area, &mode, window, cx);
        cx.notify();
    }
    /// Asset-browser → Materials routing: activating a material source
    /// (`.azmaterial.ron` / `.azmaterialtype.ron` / `.azmat.ron`) switches the
    /// workspace into Materials mode and records/opens the selection. The
    /// action always propagates so the authored-selection controller still
    /// loads the document.
    pub(crate) fn toggle_left_dock(
        &mut self,
        _action: &crate::actions::ToggleOutliner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.toggle_left_dock_state("hierarchy") {
            return;
        }
        let show = self.state.workspace_projection().show_left;
        self.set_scene_dock_open(gpui_component::dock::DockPlacement::Left, show, window, cx);
        cx.notify();
    }
    pub(crate) fn toggle_asset_browser_dock(
        &mut self,
        _action: &crate::actions::ToggleAssetBrowser,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.toggle_bottom_dock_state("assets") {
            return;
        }
        let show = self.state.workspace_projection().show_bottom;
        self.set_scene_dock_open(
            gpui_component::dock::DockPlacement::Bottom,
            show,
            window,
            cx,
        );
        cx.notify();
    }
    pub(crate) fn toggle_right_dock(
        &mut self,
        _action: &crate::actions::ToggleInspector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.toggle_right_dock_state("details") {
            return;
        }
        let show = self.state.workspace_projection().show_right;
        self.set_scene_dock_open(gpui_component::dock::DockPlacement::Right, show, window, cx);
        cx.notify();
    }
    pub(crate) fn toggle_bottom_console_dock(
        &mut self,
        _action: &crate::actions::ToggleConsole,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.toggle_bottom_dock_state("console") {
            return;
        }
        let show = self.state.workspace_projection().show_bottom;
        self.set_scene_dock_open(
            gpui_component::dock::DockPlacement::Bottom,
            show,
            window,
            cx,
        );
        cx.notify();
    }
    pub(crate) fn toggle_bottom_session_dock(
        &mut self,
        _action: &crate::actions::ToggleSessionPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.toggle_bottom_dock_state("output") {
            return;
        }
        let show = self.state.workspace_projection().show_bottom;
        self.set_scene_dock_open(
            gpui_component::dock::DockPlacement::Bottom,
            show,
            window,
            cx,
        );
        cx.notify();
    }
    fn set_scene_dock_open(
        &mut self,
        placement: gpui_component::dock::DockPlacement,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = self.dock_area.read(cx).is_dock_open(placement, cx);
        if is_open != open {
            self.dock_area.update(cx, |dock, cx| {
                dock.toggle_dock(placement, window, cx);
            });
        }
    }
    pub(crate) fn left_w(&self) -> f32 {
        self.state.workspace_projection().left_w
    }

    pub(crate) fn right_w(&self) -> f32 {
        self.state.workspace_projection().right_w
    }

    pub(crate) fn bottom_h(&self) -> f32 {
        self.state.workspace_projection().bottom_h
    }

    pub(crate) fn show_left(&self) -> bool {
        self.state.workspace_projection().show_left
    }

    pub(crate) fn show_right(&self) -> bool {
        self.state.workspace_projection().show_right
    }

    pub(crate) fn show_bottom(&self) -> bool {
        self.state.workspace_projection().show_bottom
    }

    pub(crate) fn resize_left_dock_from_start(
        &mut self,
        start_width: f32,
        delta_x: f32,
        cx: &mut Context<Self>,
    ) {
        self.state
            .resize_left_from_start_state(start_width, delta_x);
        cx.notify();
    }

    pub(crate) fn resize_right_dock_from_start(
        &mut self,
        start_width: f32,
        delta_x: f32,
        cx: &mut Context<Self>,
    ) {
        self.state
            .resize_right_from_start_state(start_width, delta_x);
        cx.notify();
    }

    pub(crate) fn resize_bottom_dock_from_start(
        &mut self,
        start_height: f32,
        delta_y: f32,
        cx: &mut Context<Self>,
    ) {
        self.state
            .resize_bottom_from_start_state(start_height, delta_y);
        cx.notify();
    }

    pub(crate) fn begin_left_dock_resize(&mut self, position: Point<Pixels>) {
        let (start_x, start_size) = self.state.begin_left_resize_state(position);
        trace_aether_ui_state(
            "resize.start",
            format!("edge=left x={:.1} start_size={:.1}", start_x, start_size),
            &self.state,
        );
    }

    pub(crate) fn begin_right_dock_resize(&mut self, position: Point<Pixels>) {
        let (start_x, start_size) = self.state.begin_right_resize_state(position);
        trace_aether_ui_state(
            "resize.start",
            format!("edge=right x={:.1} start_size={:.1}", start_x, start_size),
            &self.state,
        );
    }

    pub(crate) fn begin_bottom_dock_resize(&mut self, position: Point<Pixels>) {
        let (start_y, start_size) = self.state.begin_bottom_resize_state(position);
        trace_aether_ui_state(
            "resize.start",
            format!("edge=bottom y={:.1} start_size={:.1}", start_y, start_size),
            &self.state,
        );
    }

    pub(crate) fn resize_left_dock_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.state.resize_left_to_state(position);
        cx.notify();
    }

    pub(crate) fn resize_right_dock_to(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.state.resize_right_to_state(position);
        cx.notify();
    }

    pub(crate) fn resize_bottom_dock_to(
        &mut self,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.state.resize_bottom_to_state(position);
        cx.notify();
    }

    pub(crate) fn toggle_left_panel_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.panel_capabilities().left {
            trace_aether_ui_state(
                "panel.toggle_left_ignored",
                "view has no left panel",
                &self.state,
            );
            return;
        }
        let before = self.state.trace_summary();
        self.state.toggle_left_panel_state();
        let show = self.state.workspace_projection().show_left;
        self.set_scene_dock_open(gpui_component::dock::DockPlacement::Left, show, window, cx);
        trace_aether_ui_state("panel.toggle_left", format!("before={before}"), &self.state);
        cx.notify();
    }

    pub(crate) fn toggle_right_panel_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.panel_capabilities().right {
            trace_aether_ui_state(
                "panel.toggle_right_ignored",
                "view has no right panel",
                &self.state,
            );
            return;
        }
        let before = self.state.trace_summary();
        self.state.toggle_right_panel_state();
        let show = self.state.workspace_projection().show_right;
        self.set_scene_dock_open(gpui_component::dock::DockPlacement::Right, show, window, cx);
        trace_aether_ui_state(
            "panel.toggle_right",
            format!("before={before}"),
            &self.state,
        );
        cx.notify();
    }

    pub(crate) fn toggle_bottom_panel_visibility(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.state.panel_capabilities().bottom {
            trace_aether_ui_state(
                "panel.toggle_bottom_ignored",
                "view has no bottom panel",
                &self.state,
            );
            return;
        }
        let before = self.state.trace_summary();
        self.state.toggle_bottom_panel_state();
        let show = self.state.workspace_projection().show_bottom;
        self.set_scene_dock_open(
            gpui_component::dock::DockPlacement::Bottom,
            show,
            window,
            cx,
        );
        trace_aether_ui_state(
            "panel.toggle_bottom",
            format!("before={before}"),
            &self.state,
        );
        cx.notify();
    }
}

pub(crate) fn panel_button_style(
    active: bool,
    enabled: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("width", "26px".to_owned()),
        ("height", "20px".to_owned()),
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "center".to_owned()),
        ("borderRadius", "4px".to_owned()),
        (
            "cursor",
            if enabled { "pointer" } else { "default" }.to_owned(),
        ),
        (
            "color",
            hsla_css(if !enabled {
                theme.muted_foreground.opacity(0.45)
            } else if active {
                theme.accent
            } else {
                theme.muted_foreground
            }),
        ),
        ("opacity", if enabled { "1" } else { "0.45" }.to_owned()),
    ])
}
