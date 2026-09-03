//! Menu, preferences, and transient overlay projections.

use az_editor_ui::panels::project_workflow;
use gpui::{AppContext, Context, Pixels, Point, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherItems, AetherMenuAction, AetherStyle};
use crate::app::aether_editor_model::{
    AetherEditorState, OverlayDismissal, contains_point_in_triangle, is_view_pill_key,
    top_menu_button_width, trace_aether_ui_state, trace_value, update_editor_setting_value,
};
use crate::app::aether_editor_view::AetherEditorView;
use crate::attach::EditorAttachSession;
use crate::settings::{EditorSettings, SettingsStore, save_global_settings};

use super::super::AetherViewAction;
use super::super::presentation::{
    hsla_css, non_empty_string_or, set_item_style, settings_select_option_style,
};

impl AetherEditorView {
    pub(crate) fn menus(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        let mut menus = aether_menu_definitions();
        self.apply_collection_state("menus", &mut menus, &theme);
        menus
    }

    pub(crate) fn activate_menu_item(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if item.disabled || item.sep {
            cx.stop_propagation();
            return;
        }
        let Some(action) = item.menu_action else {
            // Rows without an implemented command are projections only. They
            // are disabled by construction and cannot fall through to the
            // generic string-key router.
            debug_assert!(false, "enabled menu row has no typed action");
            cx.stop_propagation();
            return;
        };
        self.state.close_menu_state();
        dispatch_menu_action(action, window, cx);
        cx.stop_propagation();
        cx.notify();
    }
    pub(crate) fn modal_cats(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if !self.is_settings() {
            return Vec::new();
        }
        let theme = cx.theme().clone();
        settings_modal_categories(&self.state.overlay_projection().modal_category, &theme)
    }
    pub(crate) fn modal_rows(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        if !self.is_settings() {
            return Vec::new();
        }
        let settings = self.editor_settings(cx);
        let theme = cx.theme().clone();
        let theme_options = available_theme_setting_options(cx);
        settings_modal_rows_for_category(
            &self.state.overlay_projection().modal_category,
            &settings,
            &theme_options,
            &theme,
        )
    }
    pub(crate) fn settings_select_items_for_key(
        &self,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AetherItem> {
        let settings = self.editor_settings(cx);
        let theme = cx.theme().clone();
        let theme_options = available_theme_setting_options(cx);
        settings_select_items(key, &settings, &theme_options, &theme)
    }

    pub(crate) fn open_preferences_action(
        &mut self,
        _action: &crate::actions::Preferences,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preferences_modal(cx);
        cx.notify();
    }
    pub(crate) fn dismiss_overlay_action(
        &mut self,
        _action: &crate::actions::DismissOverlay,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dismissal = self.state.dismiss_top_layer_state();
        self.apply_overlay_dismissal(dismissal, "escape", window, cx);
        cx.stop_propagation();
        cx.notify();
    }
    pub(crate) fn show_about_dialog(
        &mut self,
        _action: &crate::actions::About,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_about_modal();
        cx.notify();
    }
    pub(crate) fn on_bottom_drag<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }
    pub(crate) fn on_close<E>(&mut self, _event: &E, _window: &mut Window, cx: &mut Context<Self>) {
        self.apply_action(AetherViewAction::CloseBuildAndMenu);
        cx.notify();
    }

    pub(crate) fn on_left_drag<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }
    pub(crate) fn on_right_drag<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
    }
    pub(crate) fn on_toggle<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::ToggleBuild);
        cx.notify();
    }
    pub(crate) fn open_settings<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_preferences_modal(cx);
        cx.notify();
    }
    pub(crate) fn reset_modal<E>(
        &mut self,
        _event: &E,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_editor_settings(window, cx);
        cx.notify();
    }
    pub(crate) fn stop_prop<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let _ = self.state.inner_layer_click_state();
        cx.stop_propagation();
    }
    pub(crate) fn toggle_layout_menu<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::ToggleLayoutMenu);
        if self.state.overlay_projection().layout_open {
            crate::perf::begin_interaction(crate::perf::POPOVER_TO_VISIBLE);
        }
        cx.notify();
    }
    pub(crate) fn toggle_level_menu<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::ToggleLevelMenu);
        if self.state.overlay_projection().level_open {
            crate::perf::begin_interaction(crate::perf::POPOVER_TO_VISIBLE);
        }
        cx.notify();
    }
    pub(crate) fn toggle_pipe<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::TogglePipe);
        cx.notify();
    }

    pub(crate) fn toggle_menu_named(&mut self, name: &str, cx: &mut Context<Self>) {
        let before = self.state.trace_summary();
        if self.state.toggle_menu_state(name) {
            crate::perf::begin_interaction(crate::perf::POPOVER_TO_VISIBLE);
        }
        trace_aether_ui_state(
            "menu.toggle",
            format!("name={} before={}", trace_value(name), before),
            &self.state,
        );
        cx.notify();
    }

    pub(crate) fn track_menu_pointer(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let before = self.state.trace_summary();
        let Some(pending_menu) = self.state.track_menu_pointer_state(position) else {
            return;
        };
        trace_aether_ui_state(
            "menu.aim_release",
            format!("name={} before={}", trace_value(&pending_menu), before),
            &self.state,
        );
        cx.notify();
    }

    pub(crate) fn enter_menu_named(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.state.menu_open_state() && !self.state.menu_open_for_state(name) {
            self.switch_open_menu_from_hover(name, "menu.hover_switch", cx);
        }
    }

    fn switch_open_menu_from_hover(&mut self, name: &str, event: &str, cx: &mut Context<Self>) {
        let before = self.state.trace_summary();
        if !self.state.switch_open_menu_from_title_hover_state(name) {
            return;
        }
        trace_aether_ui_state(
            event,
            format!("name={} before={}", trace_value(name), before),
            &self.state,
        );
        cx.notify();
    }

    pub(crate) fn activate_settings_control(
        &mut self,
        item: &AetherItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match item.kind.as_str() {
            "settings-cat" => {
                self.state.select_preferences_category_state(&item.key);
            }
            "settings-select" => {
                self.state.close_select_state();
            }
            "settings-option" => {
                self.update_editor_setting(&item.key, &item.value, window, cx);
                self.state.close_select_state();
            }
            "add-component-schema" | "authored-enum-option" | "prefab-source-document" => {
                if item.disabled {
                    cx.stop_propagation();
                    return;
                }
                item.on_click(&(), window, cx);
                self.state.close_select_state();
            }
            _ => self.activate_item(item, window, cx),
        }
        cx.stop_propagation();
        cx.notify();
    }

    pub(crate) fn update_setting_text_input(
        &mut self,
        item: &AetherItem,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if item.kind == "settings-text" {
            self.update_editor_setting(&item.key, value, window, cx);
            cx.notify();
        }
    }

    pub(super) fn editor_settings(&self, cx: &mut Context<Self>) -> EditorSettings {
        if let Some(settings) = self.state.preferences_draft() {
            return settings.clone();
        }
        cx.try_global::<SettingsStore>()
            .map(|store| store.current.clone())
            .unwrap_or_default()
    }

    fn update_editor_setting(
        &mut self,
        key: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let settings = if let Some(settings) = self.state.update_preferences_draft(key, value) {
            settings
        } else if self.state.has_preferences_session() {
            return;
        } else {
            let mut settings = self.editor_settings(cx);
            if !update_editor_setting_value(&mut settings, key, value) {
                return;
            }
            settings
        };
        self.apply_editor_settings_live(settings, key, window, cx);
    }

    fn reset_editor_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.reset_preferences_presentation();
        let settings = self
            .state
            .reset_preferences_draft()
            .unwrap_or_else(EditorSettings::default);
        self.apply_editor_settings_live(settings, "reset", window, cx);
    }

    pub(super) fn apply_overlay_dismissal(
        &mut self,
        dismissal: OverlayDismissal,
        change: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match dismissal {
            OverlayDismissal::None | OverlayDismissal::Closed => {}
            OverlayDismissal::ApplySettings(settings) => {
                self.apply_editor_settings_live(settings, change, window, cx);
            }
            OverlayDismissal::PersistSettings(settings) => {
                self.apply_editor_settings_live(settings.clone(), change, window, cx);
                self.save_editor_settings(&settings, change);
            }
        }
    }

    fn save_editor_settings(&self, settings: &EditorSettings, change: &str) {
        if let Err(error) = save_global_settings(settings) {
            tracing::error!(
                target: "az_editor::settings",
                change,
                error = %error,
                "failed to persist global editor settings"
            );
        }
    }

    fn apply_editor_settings_live(
        &mut self,
        settings: EditorSettings,
        change: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = cx
            .try_global::<SettingsStore>()
            .map(|store| store.current.clone());
        if previous.as_ref() == Some(&settings) {
            return;
        }

        if cx.has_global::<SettingsStore>() {
            cx.global_mut::<SettingsStore>().replace(settings.clone());
        } else {
            cx.set_global(SettingsStore::new(settings.clone()));
        }

        let theme_changed = previous
            .as_ref()
            .is_none_or(|previous| previous.theme != settings.theme);
        let keymap_changed = previous
            .as_ref()
            .is_none_or(|previous| previous.keymap_profile != settings.keymap_profile);

        if theme_changed
            && let Err(error) =
                az_editor_ui::theme::apply_theme_setting(cx, &settings.theme, Some(window))
        {
            tracing::error!(
                target: "az_editor::settings",
                theme = %settings.theme,
                error = %error,
                "failed to apply editor theme setting"
            );
        }

        if keymap_changed {
            crate::keymaps::bind_keymap_profile(cx, &settings.keymap_profile);
        }
    }
}

pub(crate) fn aether_menu_definitions() -> Vec<AetherItem> {
    vec![
        aether_menu(
            "File",
            vec![
                aether_menu_action(
                    "new-project",
                    "New Project...",
                    "note_add",
                    crate::keymaps::ACTION_NEW_PROJECT,
                ),
                aether_menu_action(
                    "open-project",
                    "Open Project...",
                    "folder_open",
                    crate::keymaps::ACTION_OPEN_PROJECT,
                ),
                aether_menu_separator("file-open-separator"),
                aether_menu_action("save", "Save", "save", crate::keymaps::ACTION_SAVE),
                aether_menu_action("refresh-assets", "Refresh Assets", "sync", ""),
                aether_menu_action(
                    "refresh-schema-catalog",
                    "Refresh Schema Catalog",
                    "schema",
                    "",
                ),
                aether_menu_separator("file-project-separator"),
                aether_menu_action("back-to-launcher", "Back to Project Launcher", "apps", ""),
                aether_menu_action("quit", "Exit", "logout", crate::keymaps::ACTION_QUIT),
            ],
        ),
        aether_menu(
            "Edit",
            vec![
                aether_menu_action("undo", "Undo", "undo", crate::keymaps::ACTION_UNDO),
                aether_menu_action("redo", "Redo", "redo", crate::keymaps::ACTION_REDO),
                aether_menu_separator("edit-preferences-separator"),
                aether_menu_action(
                    "preferences",
                    "Preferences...",
                    "tune",
                    crate::keymaps::ACTION_PREFERENCES,
                ),
            ],
        ),
        aether_menu(
            "View",
            vec![
                aether_menu_action(
                    "toggle-outliner",
                    "Hierarchy",
                    "account_tree",
                    crate::keymaps::ACTION_TOGGLE_OUTLINER,
                ),
                aether_menu_action(
                    "toggle-inspector",
                    "Inspector",
                    "tune",
                    crate::keymaps::ACTION_TOGGLE_INSPECTOR,
                ),
                aether_menu_action(
                    "toggle-asset-browser",
                    "Asset Browser",
                    "folder",
                    crate::keymaps::ACTION_TOGGLE_ASSET_BROWSER,
                ),
                aether_menu_action(
                    "toggle-console",
                    "Console",
                    "terminal",
                    crate::keymaps::ACTION_TOGGLE_CONSOLE,
                ),
                aether_menu_action(
                    "toggle-session-panel",
                    "Session",
                    "hub",
                    crate::keymaps::ACTION_TOGGLE_SESSION_PANEL,
                ),
                aether_menu_separator("view-graph-separator"),
                aether_menu_action(
                    "toggle-graph-panel",
                    "Visual Graph Workbench",
                    "account_tree",
                    crate::keymaps::ACTION_TOGGLE_GRAPH_PANEL,
                ),
                aether_menu_separator("view-refresh-separator"),
                aether_menu_action(
                    "refresh-authored-outline",
                    "Refresh Outliner",
                    "refresh",
                    "",
                ),
                aether_menu_separator("view-window-separator"),
                aether_menu_action("reset-layout", "Reset Layout", "restart_alt", ""),
                aether_menu_action(
                    "toggle-fullscreen",
                    "Toggle Fullscreen",
                    "fullscreen",
                    crate::keymaps::ACTION_TOGGLE_FULLSCREEN,
                ),
            ],
        ),
        aether_menu(
            "Run",
            vec![
                aether_menu_action(
                    "launch-editor-world",
                    "Launch Editor World",
                    "play_arrow",
                    "",
                ),
                aether_menu_action("stop-editor-world", "Stop Editor World", "stop", ""),
                aether_menu_separator("run-refresh-separator"),
                aether_menu_action(
                    "refresh-runtime-status",
                    "Refresh Runtime Status",
                    "refresh",
                    "",
                ),
                aether_menu_action(
                    "refresh-viewport-frame",
                    "Refresh Viewport Frame",
                    "image",
                    "",
                ),
                aether_menu_action(
                    "refresh-runtime-projections",
                    "Refresh Runtime Projections",
                    "conversion_path",
                    "",
                ),
            ],
        ),
        aether_menu(
            "Session",
            vec![
                aether_menu_action("refresh-session-status", "Refresh Status", "refresh", ""),
                aether_menu_separator("session-services-separator"),
                aether_menu_action(
                    "start-session-services",
                    "Start Services",
                    "play_circle",
                    "",
                ),
                aether_menu_action("stop-session-services", "Stop Services", "stop_circle", ""),
                aether_menu_separator("session-publish-separator"),
                aether_menu_action("plan-session-publish", "Plan Publish", "publish", ""),
                aether_menu_action("recover-session", "Recover", "restart_alt", ""),
                aether_menu_action("force-recover-session", "Force Recover", "warning", ""),
                aether_menu_separator("session-merge-separator"),
                aether_menu_action("resolve-session", "Resolve Merge", "done", ""),
                aether_menu_action("abort-session-apply", "Abort Apply", "cancel", ""),
            ],
        ),
        aether_menu(
            "Help",
            vec![aether_menu_action("about", "About Azoth", "info", "")],
        ),
    ]
}

fn aether_menu(name: &str, items: Vec<AetherItem>) -> AetherItem {
    AetherItem {
        name: name.to_owned(),
        items: AetherItems(items),
        ..AetherItem::default()
    }
}

fn aether_menu_action(key: &str, label: &str, icon: &str, shortcut_action: &str) -> AetherItem {
    let menu_action = menu_action_for_key(key);
    let mut item = AetherItem {
        menu_action,
        kind: "menu-action".to_owned(),
        key: key.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        shortcut: if shortcut_action.is_empty() {
            String::new()
        } else {
            crate::keymaps::shortcut_label(crate::keymaps::KEYMAP_PROFILE_DEFAULT, shortcut_action)
                .unwrap_or_default()
        },
        disabled: menu_action.is_none(),
        notsep: true,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", menu_item_style());
    item
}

fn menu_action_for_key(key: &str) -> Option<AetherMenuAction> {
    Some(match key {
        "new-project" => AetherMenuAction::NewProject,
        "open-project" => AetherMenuAction::OpenProject,
        "save" => AetherMenuAction::Save,
        "refresh-assets" => AetherMenuAction::RefreshAssets,
        "refresh-schema-catalog" => AetherMenuAction::RefreshGameDataCatalog,
        "back-to-launcher" => AetherMenuAction::BackToProjectLauncher,
        "quit" => AetherMenuAction::Quit,
        "undo" => AetherMenuAction::Undo,
        "redo" => AetherMenuAction::Redo,
        "preferences" => AetherMenuAction::Preferences,
        "toggle-outliner" => AetherMenuAction::ToggleOutliner,
        "toggle-inspector" => AetherMenuAction::ToggleInspector,
        "toggle-asset-browser" => AetherMenuAction::ToggleAssetBrowser,
        "toggle-console" => AetherMenuAction::ToggleConsole,
        "toggle-session-panel" => AetherMenuAction::ToggleSessionPanel,
        "toggle-graph-panel" => AetherMenuAction::ToggleGraphPanel,
        "refresh-authored-outline" => AetherMenuAction::RefreshAuthoredOutline,
        "reset-layout" => AetherMenuAction::ResetLayout,
        "toggle-fullscreen" => AetherMenuAction::ToggleFullscreen,
        "launch-editor-world" => AetherMenuAction::LaunchEditorWorld,
        "stop-editor-world" => AetherMenuAction::StopEditorWorld,
        "refresh-runtime-status" => AetherMenuAction::RefreshRuntimeStatus,
        "refresh-viewport-frame" => AetherMenuAction::RefreshViewportFrame,
        "refresh-runtime-projections" => AetherMenuAction::RefreshRuntimeProjections,
        "refresh-session-status" => AetherMenuAction::RefreshSessionStatus,
        "start-session-services" => AetherMenuAction::StartSessionServices,
        "stop-session-services" => AetherMenuAction::StopSessionServices,
        "recover-session" => AetherMenuAction::RecoverSession,
        "force-recover-session" => AetherMenuAction::ForceRecoverSession,
        "about" => AetherMenuAction::About,
        // These rows describe planned commands for which the editor exposes
        // no GPUI action yet. Keeping them disabled is honest and prevents a
        // click from masquerading as a successful command.
        "plan-session-publish" | "resolve-session" | "abort-session-apply" => return None,
        unknown => {
            debug_assert!(false, "unclassified Aether menu action `{unknown}`");
            return None;
        }
    })
}

fn dispatch_menu_action(
    action: AetherMenuAction,
    window: &mut Window,
    cx: &mut Context<AetherEditorView>,
) {
    match action {
        AetherMenuAction::NewProject => {
            window.dispatch_action(Box::new(crate::actions::NewProject), cx)
        }
        AetherMenuAction::OpenProject => {
            window.dispatch_action(Box::new(crate::actions::OpenProject), cx)
        }
        AetherMenuAction::Save => window.dispatch_action(Box::new(crate::actions::Save), cx),
        AetherMenuAction::RefreshAssets => {
            window.dispatch_action(Box::new(crate::actions::RefreshAssets), cx)
        }
        AetherMenuAction::RefreshGameDataCatalog => {
            window.dispatch_action(Box::new(crate::actions::RefreshGameDataCatalog), cx)
        }
        AetherMenuAction::BackToProjectLauncher => {
            window.dispatch_action(Box::new(crate::actions::BackToProjectLauncher), cx)
        }
        AetherMenuAction::Quit => window.dispatch_action(Box::new(crate::actions::Quit), cx),
        AetherMenuAction::Undo => window.dispatch_action(Box::new(crate::actions::Undo), cx),
        AetherMenuAction::Redo => window.dispatch_action(Box::new(crate::actions::Redo), cx),
        AetherMenuAction::Preferences => {
            window.dispatch_action(Box::new(crate::actions::Preferences), cx)
        }
        AetherMenuAction::ToggleOutliner => {
            window.dispatch_action(Box::new(crate::actions::ToggleOutliner), cx)
        }
        AetherMenuAction::ToggleInspector => {
            window.dispatch_action(Box::new(crate::actions::ToggleInspector), cx)
        }
        AetherMenuAction::ToggleAssetBrowser => {
            window.dispatch_action(Box::new(crate::actions::ToggleAssetBrowser), cx)
        }
        AetherMenuAction::ToggleConsole => {
            window.dispatch_action(Box::new(crate::actions::ToggleConsole), cx)
        }
        AetherMenuAction::ToggleSessionPanel => {
            window.dispatch_action(Box::new(crate::actions::ToggleSessionPanel), cx)
        }
        AetherMenuAction::ToggleGraphPanel => {
            window.dispatch_action(Box::new(crate::actions::ToggleGraphPanel), cx)
        }
        AetherMenuAction::RefreshAuthoredOutline => {
            window.dispatch_action(Box::new(crate::actions::RefreshAuthoredOutline), cx)
        }
        AetherMenuAction::ResetLayout => {
            window.dispatch_action(Box::new(crate::actions::ResetLayout), cx)
        }
        AetherMenuAction::ToggleFullscreen => {
            window.dispatch_action(Box::new(crate::actions::ToggleFullscreen), cx)
        }
        AetherMenuAction::LaunchEditorWorld => {
            window.dispatch_action(Box::new(crate::actions::LaunchEditorWorld), cx)
        }
        AetherMenuAction::StopEditorWorld => window.dispatch_action(
            Box::new(crate::actions::StopEditorWorld { preserve: false }),
            cx,
        ),
        AetherMenuAction::RefreshRuntimeStatus => {
            window.dispatch_action(Box::new(crate::actions::RefreshRuntimeStatus), cx)
        }
        AetherMenuAction::RefreshViewportFrame => {
            window.dispatch_action(Box::new(crate::actions::RefreshViewportFrame), cx)
        }
        AetherMenuAction::RefreshRuntimeProjections => {
            window.dispatch_action(Box::new(crate::actions::RefreshRuntimeProjections), cx)
        }
        AetherMenuAction::RefreshSessionStatus => {
            window.dispatch_action(Box::new(crate::actions::RefreshSessionStatus), cx)
        }
        AetherMenuAction::StartSessionServices => {
            window.dispatch_action(Box::new(crate::actions::StartSessionServices), cx)
        }
        AetherMenuAction::StopSessionServices => {
            window.dispatch_action(Box::new(crate::actions::StopSessionServices), cx)
        }
        AetherMenuAction::RecoverSession => {
            window.dispatch_action(Box::new(crate::actions::RecoverSession), cx)
        }
        AetherMenuAction::ForceRecoverSession => {
            window.dispatch_action(Box::new(crate::actions::ForceRecoverSession), cx)
        }
        AetherMenuAction::About => window.dispatch_action(Box::new(crate::actions::About), cx),
    }
}

fn aether_menu_separator(key: &str) -> AetherItem {
    AetherItem {
        key: key.to_owned(),
        sep: true,
        notsep: false,
        ..AetherItem::default()
    }
}

fn menu_item_style() -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "space-between".to_owned()),
        ("height", "27px".to_owned()),
        ("padding", "0 9px".to_owned()),
        ("borderRadius", "4px".to_owned()),
        ("fontSize", "11.5px".to_owned()),
        ("cursor", "pointer".to_owned()),
    ])
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingOption {
    pub(crate) value: String,
    pub(crate) label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SettingsRowProjection {
    pub(crate) key: String,
    pub(crate) label: String,
    pub(crate) description: String,
    pub(crate) value: String,
    pub(crate) control: SettingsControlProjection,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsControlProjection {
    Select,
    Text,
}

pub(crate) fn settings_row_projection(
    category: &str,
    settings: &EditorSettings,
    theme_options: &[SettingOption],
) -> Vec<SettingsRowProjection> {
    match category {
        "appearance" => vec![SettingsRowProjection {
            key: "theme".to_owned(),
            label: "Theme".to_owned(),
            description: "Applies immediately; Done saves, Cancel reverts".to_owned(),
            value: selected_option_label(theme_options, &settings.theme)
                .unwrap_or_else(|| humanize_theme_setting(&settings.theme)),
            control: SettingsControlProjection::Select,
        }],
        "keymap" => vec![SettingsRowProjection {
            key: "keymap_profile".to_owned(),
            label: "Keymap Profile".to_owned(),
            description: "Applies immediately; only Default is currently registered".to_owned(),
            value: keymap_profile_label(&settings.keymap_profile),
            control: SettingsControlProjection::Select,
        }],
        "source-navigation" => vec![SettingsRowProjection {
            key: "source_navigation.file_url_template".to_owned(),
            label: "File URL Template".to_owned(),
            description: "Applies immediately to source links; Done saves, Cancel reverts"
                .to_owned(),
            value: settings.source_navigation.file_url_template.clone(),
            control: SettingsControlProjection::Text,
        }],
        _ => Vec::new(),
    }
}

fn settings_modal_categories(
    selected: &str,
    theme: &gpui_component::theme::Theme,
) -> Vec<AetherItem> {
    [
        ("appearance", "Appearance", "palette"),
        ("keymap", "Keymap", "keyboard"),
        ("source-navigation", "Source Navigation", "open_in_new"),
    ]
    .into_iter()
    .map(|(key, label, icon)| {
        let active = selected == key;
        let mut item = AetherItem {
            kind: "settings-cat".to_owned(),
            key: key.to_owned(),
            label: label.to_owned(),
            icon: icon.to_owned(),
            active,
            selected: active,
            ..AetherItem::default()
        };
        set_item_style(&mut item, "style", settings_category_style(active, theme));
        item
    })
    .collect()
}

fn settings_modal_rows_for_category(
    category: &str,
    settings: &EditorSettings,
    theme_options: &[SettingOption],
    theme: &gpui_component::theme::Theme,
) -> Vec<AetherItem> {
    settings_row_projection(category, settings, theme_options)
        .into_iter()
        .map(|row| settings_row_item(row, theme))
        .collect()
}

fn settings_row_item(
    row: SettingsRowProjection,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let is_select = matches!(row.control, SettingsControlProjection::Select);
    let is_text = matches!(row.control, SettingsControlProjection::Text);
    let mut item = AetherItem {
        kind: if is_select {
            "settings-select".to_owned()
        } else {
            "settings-text".to_owned()
        },
        key: row.key,
        label: row.label,
        sub: row.description,
        desc: true,
        value: row.value,
        is_row: true,
        is_select,
        is_text,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", settings_row_style(theme));
    item
}

fn settings_select_items(
    key: &str,
    settings: &EditorSettings,
    theme_options: &[SettingOption],
    theme: &gpui_component::theme::Theme,
) -> Vec<AetherItem> {
    let options = match key {
        "theme" => theme_options.to_vec(),
        "keymap_profile" => crate::keymaps::keymap_profiles()
            .iter()
            .map(|profile| SettingOption {
                value: profile.key.to_owned(),
                label: profile.label.to_owned(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let selected_value = match key {
        "theme" => settings.theme.as_str(),
        "keymap_profile" => settings.keymap_profile.as_str(),
        _ => "",
    };
    options
        .into_iter()
        .map(|option| {
            let selected = if key == "theme" {
                theme_setting_matches(&option.value, selected_value)
            } else {
                option.value == selected_value
            };
            let mut item = AetherItem {
                kind: "settings-option".to_owned(),
                key: key.to_owned(),
                label: option.label,
                value: option.value,
                selected,
                ..AetherItem::default()
            };
            set_item_style(
                &mut item,
                "style",
                settings_select_option_style(selected, theme),
            );
            item
        })
        .collect()
}

fn available_theme_setting_options(cx: &mut Context<AetherEditorView>) -> Vec<SettingOption> {
    let mut options = vec![
        SettingOption {
            value: "system".to_owned(),
            label: "System".to_owned(),
        },
        SettingOption {
            value: "default-dark".to_owned(),
            label: "Default Dark".to_owned(),
        },
        SettingOption {
            value: "default-light".to_owned(),
            label: "Default Light".to_owned(),
        },
    ];
    let mut themes = az_editor_ui::theme::available_themes(cx);
    themes.sort_by(|left, right| left.name.cmp(&right.name));
    for theme in themes {
        let label = title_case_identifier(&theme.name);
        if theme.has_dark {
            options.push(SettingOption {
                value: format!("{}:dark", theme.name),
                label: format!("{label} Dark"),
            });
        }
        if theme.has_light {
            options.push(SettingOption {
                value: format!("{}:light", theme.name),
                label: format!("{label} Light"),
            });
        }
    }
    options
}

fn selected_option_label(options: &[SettingOption], value: &str) -> Option<String> {
    options
        .iter()
        .find(|option| theme_setting_matches(&option.value, value) || option.value == value)
        .map(|option| option.label.clone())
}

fn keymap_profile_label(key: &str) -> String {
    crate::keymaps::keymap_profiles()
        .iter()
        .find(|profile| profile.key.eq_ignore_ascii_case(key))
        .map(|profile| profile.label.to_owned())
        .unwrap_or_else(|| non_empty_string_or(title_case_identifier(key), key))
}

fn humanize_theme_setting(setting: &str) -> String {
    if setting.trim().is_empty() {
        return "System".to_owned();
    }
    let mut parts = setting.split([':', '/', '@']).collect::<Vec<_>>();
    if parts.len() == 2 {
        let mode = parts.pop().unwrap_or_default();
        let name = parts.pop().unwrap_or_default();
        return format!(
            "{} {}",
            title_case_identifier(name),
            title_case_identifier(mode)
        );
    }
    title_case_identifier(setting)
}

fn title_case_identifier(value: &str) -> String {
    let words = value
        .split(['-', '_', '.', ':', '/', '\\'])
        .filter(|word| !word.trim().is_empty())
        .collect::<Vec<_>>();
    if words.is_empty() {
        return String::new();
    }
    words
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first
                    .to_uppercase()
                    .chain(chars.flat_map(char::to_lowercase))
                    .collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn theme_setting_matches(left: &str, right: &str) -> bool {
    theme_setting_key(left) == theme_setting_key(right)
}

fn theme_setting_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn settings_category_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "10px".to_owned()),
        ("height", "32px".to_owned()),
        ("padding", "0 11px".to_owned()),
        ("borderRadius", "7px".to_owned()),
        ("fontSize", "12px".to_owned()),
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
                theme.accent.opacity(0.16)
            } else {
                theme.transparent
            }),
        ),
    ])
}

fn settings_row_style(theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("color", hsla_css(theme.foreground)),
        (
            "borderBottom",
            format!("1px solid {}", hsla_css(theme.border)),
        ),
    ])
}

pub(super) fn select_popover_style(
    theme: &gpui_component::theme::Theme,
    add_component: bool,
) -> AetherStyle {
    let (top, left, right, width) = if add_component {
        ("112px", "auto", "28px", "320px")
    } else {
        ("156px", "50%", "auto", "280px")
    };
    AetherStyle::from_pairs(&[
        ("position", "absolute".to_owned()),
        ("top", top.to_owned()),
        ("left", left.to_owned()),
        ("right", right.to_owned()),
        ("width", width.to_owned()),
        ("padding", "6px".to_owned()),
        ("background", hsla_css(theme.popover)),
        ("border", format!("1px solid {}", hsla_css(theme.border))),
        ("borderRadius", "8px".to_owned()),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceControlSegment {
    pub(crate) branch: String,
    pub(crate) change_count: usize,
}

pub(crate) fn source_control_projection(
    session: &EditorAttachSession,
) -> Option<SourceControlSegment> {
    let branch = session.source_status.branch.as_deref()?.trim();
    if branch.is_empty() {
        return None;
    }
    Some(SourceControlSegment {
        branch: branch.to_owned(),
        change_count: session.source_status.changed_lines.len(),
    })
}

pub(super) fn menu_button_style(open: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("height", "34px".to_owned()),
        ("padding", "0 9px".to_owned()),
        ("fontSize", "12px".to_owned()),
        ("cursor", "pointer".to_owned()),
        (
            "color",
            hsla_css(if open {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            hsla_css(if open {
                theme.secondary
            } else {
                theme.transparent
            }),
        ),
    ])
}
