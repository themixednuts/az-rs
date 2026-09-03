// Stable Aether editor view.
//
// This file began as htmlswap GPUI output, but it is now adopted editor code.
// Do not replace it wholesale with regenerated output. Treat new DC/htmlswap
// output as a reference by region and port changes into the owned view/model.
// Keep missing engine-backed areas explicit in the UI instead of silently
// reintroducing mock generated fallbacks.
//
// Asset requirement: Material Symbol SVGs at `icons/dc/*.svg`.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AppContext as _, Context, DragMoveEvent, Empty, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
};
use gpui_component::ActiveTheme as _;
use gpui_component::Disableable as _;
use gpui_component::IconName;
use gpui_component::Sizable as _;
use gpui_component::StyledExt as _;
use gpui_component::button::Button;
use gpui_component::button::ButtonVariants as _;
use gpui_component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_component::scroll::ScrollableElement as _;

use super::aether_common::{AetherItem, AetherStyle};
use super::aether_editor_model::{AetherEditorState, trace_aether_ui_interaction};

pub struct AetherEditorView {
    pub(crate) state: AetherEditorState,
    /// Profile-driven GPUI dock workspace for scene mode (left/center/right/bottom).
    pub(crate) dock_area: gpui::Entity<gpui_component::dock::DockArea>,
    asset_search_input: gpui::Entity<gpui_component::input::InputState>,
    asset_create_name_input: gpui::Entity<gpui_component::input::InputState>,
    asset_create_folder_input: gpui::Entity<gpui_component::input::InputState>,
    asset_rename_path_input: gpui::Entity<gpui_component::input::InputState>,
    add_component_search_input: gpui::Entity<gpui_component::input::InputState>,
    console_filter_input: gpui::Entity<gpui_component::input::InputState>,
    titlebar_region: gpui::Entity<AetherTitlebarRegion>,
    toolbar_region: gpui::Entity<AetherToolbarRegion>,
    activity_region: gpui::Entity<AetherActivityRailRegion>,
    workspace_region: gpui::Entity<AetherWorkspaceRegion>,
    status_region: gpui::Entity<AetherStatusRegion>,
    modal_region: gpui::Entity<AetherModalRegion>,
    _htmlswap_subscriptions: Vec<gpui::Subscription>,
    _htmlswap_component_text_inputs:
        std::collections::BTreeMap<String, gpui::Entity<gpui_component::input::InputState>>,
    _htmlswap_component_text_input_placeholders: std::collections::BTreeMap<String, String>,
}

impl AetherEditorView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let dock_area = crate::workspace::dock::create_default_dock_area(window, cx);
        let initial_project_state = crate::ui_state_persistence::initial_project_state(cx);
        let editor = cx.entity().downgrade();
        let mut editor_state = AetherEditorState::new();
        editor_state.restore_mode(&initial_project_state.last_mode);
        let mut this = Self {
            state: editor_state,
            dock_area: dock_area.clone(),
            asset_search_input: cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("Search assets...")
            }),
            asset_create_name_input: cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("asset_name")
            }),
            asset_create_folder_input: cx
                .new(|cx| gpui_component::input::InputState::new(window, cx).placeholder("folder")),
            asset_rename_path_input: cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx)
                    .placeholder("folder/asset_name.ext")
            }),
            add_component_search_input: cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx)
                    .placeholder("Search components...")
            }),
            console_filter_input: cx.new(|cx| {
                gpui_component::input::InputState::new(window, cx).placeholder("Filter...")
            }),
            titlebar_region: cx.new(|_| AetherTitlebarRegion {
                editor: editor.clone(),
            }),
            toolbar_region: cx.new(|_| AetherToolbarRegion {
                editor: editor.clone(),
            }),
            activity_region: cx.new(|_| AetherActivityRailRegion {
                editor: editor.clone(),
            }),
            workspace_region: cx.new(|_| AetherWorkspaceRegion {
                dock_area: dock_area.clone(),
            }),
            status_region: cx.new(|_| AetherStatusRegion {
                editor: editor.clone(),
            }),
            modal_region: cx.new(|_| AetherModalRegion { editor }),
            _htmlswap_subscriptions: Vec::new(),
            _htmlswap_component_text_inputs: std::collections::BTreeMap::new(),
            _htmlswap_component_text_input_placeholders: std::collections::BTreeMap::new(),
        };
        let htmlswap_subscription = cx.subscribe_in(
            &this.asset_search_input,
            window,
            |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.asset_on_search(&value, window, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(htmlswap_subscription);
        let htmlswap_subscription = cx.subscribe_in(
            &this.asset_create_name_input,
            window,
            |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.asset_create_on_name(&value, window, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(htmlswap_subscription);
        let htmlswap_subscription = cx.subscribe_in(
            &this.asset_create_folder_input,
            window,
            |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.asset_create_on_folder(&value, window, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(htmlswap_subscription);
        let htmlswap_subscription = cx.subscribe_in(
            &this.asset_rename_path_input,
            window,
            |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.asset_rename_on_path(&value, window, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(htmlswap_subscription);
        let htmlswap_subscription = cx.subscribe_in(
            &this.add_component_search_input,
            window,
            |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.update_add_component_search(&value, window, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(htmlswap_subscription);
        let htmlswap_subscription = cx.subscribe_in(
            &this.console_filter_input,
            window,
            |this: &mut Self, input, event: &gpui_component::input::InputEvent, window, cx| {
                if matches!(event, gpui_component::input::InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    this.console_on_filter(&value, window, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(htmlswap_subscription);
        let ui_state_subscription = cx.subscribe_in(
            &dock_area,
            window,
            |this: &mut Self, _, event: &gpui_component::dock::DockEvent, _, cx| {
                if matches!(event, gpui_component::dock::DockEvent::LayoutChanged) {
                    this.persist_workspace_state(false, cx);
                }
            },
        );
        this._htmlswap_subscriptions.push(ui_state_subscription);
        if let Some(asset_browser) = crate::workspace::dock::cached_asset_browser(cx) {
            let ui_state_subscription = cx.subscribe_in(
                &asset_browser,
                window,
                |this: &mut Self, _, event: &gpui_component::dock::PanelEvent, _, cx| {
                    if matches!(event, gpui_component::dock::PanelEvent::LayoutChanged) {
                        this.persist_workspace_state(false, cx);
                    }
                },
            );
            this._htmlswap_subscriptions.push(ui_state_subscription);
        }
        this
    }

    pub(crate) fn notify_mode_regions(&self, cx: &mut Context<Self>) {
        self.toolbar_region.update(cx, |_, cx| cx.notify());
        self.titlebar_region.update(cx, |_, cx| cx.notify());
        self.status_region.update(cx, |_, cx| cx.notify());
        self.activity_region.update(cx, |_, cx| cx.notify());
    }

    fn render_titlebar_region(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let level_name = self.active_level_name(cx);
        let levels = self.level_items(cx);
        let level_actions = self.level_actions(cx);
        gpui_component::TitleBar::new()
            .h(gpui::px(34.0))
            .px(gpui::px(8.0))
            .flex()
            .items_center()
            .bg(theme.title_bar)
            .border_b(gpui::px(1.0))
            .border_color(theme.title_bar_border)
            .child(
                gpui::div()
                    .w(gpui::px(13.0))
                    .h(gpui::px(13.0))
                    .rounded(gpui::px(2.0))
                    .bg(theme.accent)
                    .mr(gpui::px(10.0)),
            )
            .children(
                self.menus(cx)
                    .iter()
                    .enumerate()
                    .map(|(index, menu)| render_top_menu_dropdown(cx, index, menu)),
            )
            .child(
                gpui::div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(gpui::px(8.0))
                    .px(gpui::px(12.0))
                    .child(
                        gpui::div()
                            .min_w_0()
                            .max_w(gpui::px(220.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(gpui::px(11.0))
                            .text_color(theme.muted_foreground)
                            .child(self.project_file_title(cx)),
                    )
                    .child(
                        gpui::div()
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .child("—"),
                    )
                    .child(
                        Button::new("aether-titlebar-level")
                            .ghost()
                            .small()
                            .label(level_name)
                            .dropdown_caret(true)
                            .on_click(|_, _, _| {
                                crate::perf::begin_interaction(crate::perf::POPOVER_TO_VISIBLE);
                            })
                            .dropdown_menu(move |mut menu, _, _| {
                                menu = menu.item(PopupMenuItem::label(format!(
                                    "Open Level · {}",
                                    levels.len()
                                )));
                                if levels.is_empty() {
                                    menu = menu.item(
                                        PopupMenuItem::new("No level documents found").disabled(true),
                                    );
                                }
                                for level in &levels {
                                    let document_id = level.id.clone();
                                    let label = if level.active {
                                        format!("{}    OPEN", level.name)
                                    } else {
                                        level.name.clone()
                                    };
                                    menu = menu.item(
                                        PopupMenuItem::new(label)
                                            .checked(level.active)
                                            .disabled(level.disabled)
                                            .on_click(move |_, window, cx| {
                                                window.dispatch_action(
                                                    Box::new(
                                                        az_editor_ui::actions::SelectAuthoredDocument {
                                                            document_id: document_id.clone(),
                                                        },
                                                    ),
                                                    cx,
                                                );
                                            }),
                                    );
                                }
                                menu = menu.item(PopupMenuItem::separator());
                                for action in &level_actions {
                                    menu = match action.key.as_str() {
                                        "new-level" => menu.item(
                                            PopupMenuItem::new("New Level...").on_click(
                                                |_, window, cx| {
                                                    window.dispatch_action(
                                                        Box::new(
                                                            az_editor_ui::actions::CreateAuthoredDocument {
                                                                root_schema: az_prefab::SCENE_SOURCE_TYPE
                                                                    .to_owned(),
                                                            },
                                                        ),
                                                        cx,
                                                    );
                                                },
                                            ),
                                        ),
                                        "save-level" => menu.item(
                                            PopupMenuItem::new("Save Level").disabled(true),
                                        ),
                                        "refresh-levels" => menu.item(
                                            PopupMenuItem::new("Refresh Levels").on_click(
                                                |_, window, cx| {
                                                    window.dispatch_action(
                                                        Box::new(
                                                            az_editor_ui::actions::RefreshAuthoredOutline,
                                                        ),
                                                        cx,
                                                    );
                                                },
                                            ),
                                        ),
                                        _ => menu,
                                    };
                                }
                                menu
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_toolbar_region(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let (prefix, breadcrumb, items) = if self.mode_scene() {
            ("scene", None, self.tools(cx))
        } else if self.mode_materials() {
            (
                "materials",
                Some("Material Graph".to_owned()),
                self.material_tool_items(),
            )
        } else if self.mode_scripting() {
            (
                "scripting",
                Some("Script Graph".to_owned()),
                self.script_tool_items(),
            )
        } else if self.mode_data() {
            (
                "gamedata",
                Some("GameData".to_owned()),
                self.data_tool_items(),
            )
        } else if self.mode_sequencer() {
            (
                "sequencer",
                Some("Sequencer".to_owned()),
                self.sequencer_tool_items(),
            )
        } else {
            (
                "animation",
                Some(self.animation_toolbar_label(cx)),
                self.animation_tool_items(),
            )
        };
        let editor = cx.entity().downgrade();
        let play_editor = editor.clone();
        let scene_tools = cx
            .try_global::<az_editor_ui::EditorSceneToolState>()
            .cloned()
            .unwrap_or_default();
        let build_catalog = cx
            .try_global::<az_editor_ui::project_build::EditorProjectBuildCatalog>()
            .cloned()
            .unwrap_or_default();
        let build_state = cx
            .try_global::<az_editor_ui::project_build::EditorProjectBuildState>()
            .cloned()
            .unwrap_or_default();
        let build_profile = self.build_config_short(cx);
        let build_busy = self.build_busy(cx);
        gpui::div()
            .id("aether-toolbar-region")
            .h(gpui::px(42.0))
            .px(gpui::px(8.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(gpui::px(6.0))
            .bg(theme.tab_bar)
            .border_b(gpui::px(1.0))
            .border_color(theme.border)
            .child(toolbar_action_button(
                "aether-toolbar-save",
                "save",
                "Save",
                ToolbarAction::Save,
                &theme,
            ))
            .child(toolbar_action_button(
                "aether-toolbar-undo",
                "undo",
                "Undo",
                ToolbarAction::Undo,
                &theme,
            ))
            .child(toolbar_action_button(
                "aether-toolbar-redo",
                "redo",
                "Redo",
                ToolbarAction::Redo,
                &theme,
            ))
            .child(
                gpui::div()
                    .h(gpui::px(22.0))
                    .border_l(gpui::px(1.0))
                    .border_color(theme.border),
            )
            .child(render_mode_toolbar_strip(cx, prefix, breadcrumb, items))
            .when(self.mode_scene(), |this| {
                this.child(scene_snap_chip(
                    "aether-toolbar-grid-snap",
                    "grid_on",
                    scene_tools.grid_step_label(),
                    scene_tools.grid_snap.enabled,
                    ToolbarAction::GridSnap,
                    &theme,
                ))
                .child(scene_snap_chip(
                    "aether-toolbar-angle-snap",
                    "architecture",
                    format!("{}°", scene_tools.angle_degrees_label()),
                    scene_tools.angle_snap.enabled,
                    ToolbarAction::AngleSnap,
                    &theme,
                ))
            })
            .child(gpui::div().flex_1())
            .child(
                gpui::div()
                    .id("aether-toolbar-play")
                    .w(gpui::px(30.0))
                    .h(gpui::px(28.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(gpui::px(5.0))
                    .bg(theme.secondary)
                    .text_color(theme.foreground)
                    .cursor_pointer()
                    .on_click(move |event, window, cx| {
                        let _ = play_editor.update(cx, |editor, cx| {
                            editor.on_play_click(event, window, cx);
                        });
                    })
                    .child(htmlswap_material_symbol_icon(self.play_icon(cx))),
            )
            .child(gpui::div().flex_1())
            .child(
                Button::new("aether-toolbar-build")
                    .primary()
                    .small()
                    .icon(IconName::Hammer)
                    .label(format!("{} · {build_profile}", self.build_button_label(cx)))
                    .dropdown_caret(true)
                    .loading(build_busy)
                    .tooltip(self.build_tooltip(cx))
                    .on_click(|_, _, _| {
                        crate::perf::begin_interaction(crate::perf::POPOVER_TO_VISIBLE);
                    })
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                        menu = menu.label("Configuration");
                        for profile in &build_catalog.profiles {
                            let profile_name = profile.name.clone();
                            menu = menu.item(
                                PopupMenuItem::new(profile.name.clone())
                                    .checked(
                                        build_state.selected_profile.as_deref()
                                            == Some(profile.name.as_str()),
                                    )
                                    .on_click(move |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(
                                                az_editor_ui::actions::SetProjectBuildProfile {
                                                    profile: profile_name.clone(),
                                                },
                                            ),
                                            cx,
                                        );
                                    }),
                            );
                        }
                        menu = menu.separator().label("Target");
                        for target in &build_catalog.targets {
                            let target_key = target.key.clone();
                            menu = menu.item(
                                PopupMenuItem::new(target.label.clone())
                                    .checked(
                                        build_state.selected_target_key.as_deref()
                                            == Some(target.key.as_str()),
                                    )
                                    .on_click(move |_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(
                                                az_editor_ui::actions::SetProjectBuildTarget {
                                                    target_key: target_key.clone(),
                                                },
                                            ),
                                            cx,
                                        );
                                    }),
                            );
                        }
                        menu.separator()
                            .menu(
                                "Build",
                                Box::new(az_editor_ui::actions::ExecuteProjectBuild),
                            )
                            .menu(
                                "Plan Build",
                                Box::new(az_editor_ui::actions::PlanProjectBuild),
                            )
                            .menu(
                                "Build Settings…",
                                Box::new(az_editor_ui::actions::Preferences),
                            )
                    }),
            )
            .into_any_element()
    }

    fn render_status_region(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let warnings = self.warn_count(cx);
        let errors = self.err_count(cx);
        let controller_failures = crate::controller_set::controller_failure_presentations(cx);
        gpui::div()
            .id("aether-status-region")
            .h(gpui::px(27.0))
            .px(gpui::px(8.0))
            .flex_none()
            .flex()
            .items_center()
            .gap(gpui::px(12.0))
            .bg(theme.title_bar)
            .border_t(gpui::px(1.0))
            .border_color(theme.border)
            .text_size(gpui::px(10.5))
            .text_color(theme.muted_foreground)
            .child(self.pipe_summary(cx))
            .child(self.scm_branch(cx))
            .child(gpui::div().flex_1())
            .children(controller_failures.into_iter().map(|failure| {
                let retry = failure.retry;
                Button::new(format!("aether-controller-retry-{}", failure.kind.name()))
                    .ghost()
                    .small()
                    .label(format!("{} failed · Retry", failure.kind.name()))
                    .tooltip(failure.message)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(retry.clone()), cx);
                    })
            }))
            .when(warnings != "0", |this| {
                this.child(
                    gpui::div()
                        .text_color(theme.warning)
                        .child(format!("{warnings} warnings")),
                )
            })
            .when(errors != "0", |this| {
                this.child(
                    gpui::div()
                        .text_color(theme.danger)
                        .child(format!("{errors} errors")),
                )
            })
            .into_any_element()
    }

    fn render_activity_region(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let editor = cx.entity().downgrade();
        let settings_editor = editor.clone();
        let mode_theme = theme.clone();
        gpui::div()
            .id("aether-activity-rail-region")
            .w(gpui::px(42.0))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .items_center()
            .gap(gpui::px(4.0))
            .py(gpui::px(6.0))
            .bg(theme.title_bar)
            .border_r(gpui::px(1.0))
            .border_color(theme.border)
            .children(self.modes(cx).into_iter().map(move |mode| {
                let editor = editor.clone();
                let key = mode.key.clone();
                gpui::div()
                    .id(format!("aether-mode-{key}"))
                    .w(gpui::px(32.0))
                    .h(gpui::px(32.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(gpui::px(5.0))
                    .text_size(gpui::px(18.0))
                    .text_color(if mode.active {
                        mode_theme.accent
                    } else {
                        mode_theme.muted_foreground
                    })
                    .when(mode.active, |this| this.bg(mode_theme.list_active))
                    .cursor_pointer()
                    .hover(|this| {
                        this.bg(mode_theme.list_hover)
                            .text_color(mode_theme.foreground)
                    })
                    .on_click(move |_, window, cx| {
                        crate::perf::begin_interaction(crate::perf::ACTIVITY_MODE_TO_WORKSPACE);
                        let _ = editor.update(cx, |editor, cx| {
                            editor.set_mode_with_window(&key, window, cx);
                        });
                        cx.stop_propagation();
                    })
                    .child(htmlswap_material_symbol_icon(mode.icon))
            }))
            .child(gpui::div().flex_1())
            .child(activity_rail_button(
                "aether-rail-help",
                "help",
                "Help / About",
                move |_, window, cx| {
                    window.dispatch_action(Box::new(az_editor_ui::actions::About), cx);
                },
                &theme,
            ))
            .child(activity_rail_button(
                "aether-rail-settings",
                "settings",
                "Settings",
                move |event, window, cx| {
                    let _ = settings_editor.update(cx, |editor, cx| {
                        editor.open_settings(event, window, cx);
                    });
                },
                &theme,
            ))
            .into_any_element()
    }

    fn render_modal_region(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if self.asset_create_open() {
            return self.render_asset_create_modal(window, cx);
        }
        if self.asset_rename_open() {
            return self.render_asset_rename_modal(window, cx);
        }
        if self.asset_delete_open() {
            return self.render_asset_delete_modal(window, cx);
        }
        if !self.modal_open() {
            return gpui::div().hidden().into_any_element();
        }
        if self.is_settings() {
            return self.render_preferences_modal(window, cx);
        }
        let theme = cx.theme().clone();
        let editor = cx.entity().downgrade();
        gpui::div()
            .absolute()
            .inset(gpui::px(0.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background.opacity(0.82))
            .child(
                gpui::div()
                    .w(gpui::px(620.0))
                    .p(gpui::px(20.0))
                    .rounded(gpui::px(8.0))
                    .border(gpui::px(1.0))
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.foreground)
                    .child(self.modal_title())
                    .child(self.modal_subtitle())
                    .child(
                        gpui::div()
                            .id("aether-modal-close")
                            .mt(gpui::px(18.0))
                            .h(gpui::px(32.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(gpui::px(5.0))
                            .bg(theme.primary)
                            .cursor_pointer()
                            .on_click(move |event, window, cx| {
                                let _ = editor.update(cx, |editor, cx| {
                                    editor.close_modal(event, window, cx);
                                });
                            })
                            .child("Close"),
                    ),
            )
            .into_any_element()
    }

    fn render_preferences_modal(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = cx.theme().clone();
        let categories = self.modal_cats(cx);
        let rows = self
            .modal_rows(cx)
            .into_iter()
            .map(|row| {
                let options = if row.is_select {
                    self.settings_select_items_for_key(&row.key, cx)
                } else {
                    Vec::new()
                };
                (row, options)
            })
            .collect::<Vec<_>>();
        let editor = cx.entity().downgrade();
        let cancel_editor = editor.clone();
        let reset_editor = editor.clone();
        let done_editor = editor.clone();

        gpui::div()
            .absolute()
            .inset(gpui::px(0.0))
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.background.opacity(0.82))
            .child(
                gpui::div()
                    .w(gpui::px(680.0))
                    .h(gpui::px(430.0))
                    .flex()
                    .flex_col()
                    .rounded(gpui::px(8.0))
                    .border(gpui::px(1.0))
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.foreground)
                    .child(
                        gpui::div()
                            .h(gpui::px(48.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .px(gpui::px(18.0))
                            .border_b(gpui::px(1.0))
                            .border_color(theme.border)
                            .text_size(gpui::px(16.0))
                            .font_semibold()
                            .child("Preferences"),
                    )
                    .child(
                        gpui::div()
                            .flex_1()
                            .min_h(gpui::px(0.0))
                            .flex()
                            .child(
                                gpui::div()
                                    .w(gpui::px(176.0))
                                    .h_full()
                                    .flex_none()
                                    .flex()
                                    .flex_col()
                                    .gap(gpui::px(4.0))
                                    .p(gpui::px(10.0))
                                    .border_r(gpui::px(1.0))
                                    .border_color(theme.border)
                                    .children(categories.into_iter().map({
                                        let editor = editor.clone();
                                        let theme = theme.clone();
                                        move |category| {
                                            let editor = editor.clone();
                                            let category_action = category.clone();
                                            gpui::div()
                                                .id(format!("preferences-category-{}", category.key))
                                                .h(gpui::px(30.0))
                                                .flex()
                                                .items_center()
                                                .px(gpui::px(10.0))
                                                .rounded(gpui::px(4.0))
                                                .text_color(if category.active {
                                                    theme.foreground
                                                } else {
                                                    theme.muted_foreground
                                                })
                                                .when(category.active, |this| {
                                                    this.bg(theme.list_active)
                                                })
                                                .hover(|this| this.bg(theme.list_hover))
                                                .cursor_pointer()
                                                .child(category.label)
                                                .on_click(move |_, window, cx| {
                                                    let _ = editor.update(cx, |editor, cx| {
                                                        editor.activate_settings_control(
                                                            &category_action,
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                })
                                        }
                                    })),
                            )
                            .child(
                                gpui::div()
                                    .flex_1()
                                    .min_w(gpui::px(0.0))
                                    .h_full()
                                    .p(gpui::px(18.0))
                                    .overflow_y_scrollbar()
                                    .children(rows.into_iter().map({
                                        let editor = editor.clone();
                                        let theme = theme.clone();
                                        move |(row, options)| {
                                            let row_key = row.key.clone();
                                            let row_value = row.value.clone();
                                            let editor_for_menu = editor.clone();
                                            gpui::div()
                                                .w_full()
                                                .flex()
                                                .items_center()
                                                .justify_between()
                                                .gap(gpui::px(16.0))
                                                .py(gpui::px(8.0))
                                                .child(
                                                    gpui::div()
                                                        .flex_1()
                                                        .text_color(theme.foreground)
                                                        .child(row.label),
                                                )
                                                .when(row.is_select, |this| {
                                                    this.child(
                                                        Button::new(format!(
                                                            "preferences-select-{row_key}"
                                                        ))
                                                        .outline()
                                                        .small()
                                                        .label(row_value)
                                                        .dropdown_caret(true)
                                                        .on_click(|_, _, _| {
                                                            crate::perf::begin_interaction(
                                                                crate::perf::POPOVER_TO_VISIBLE,
                                                            );
                                                        })
                                                        .dropdown_menu(move |mut menu, _, _| {
                                                            for option in &options {
                                                                let option_action = option.clone();
                                                                let editor = editor_for_menu.clone();
                                                                menu = menu.item(
                                                                    PopupMenuItem::new(
                                                                        option.label.clone(),
                                                                    )
                                                                    .checked(option.selected)
                                                                    .on_click(
                                                                        move |_, window, cx| {
                                                                            let _ = editor.update(
                                                                                cx,
                                                                                |editor, cx| {
                                                                                    editor.activate_settings_control(
                                                                                        &option_action,
                                                                                        window,
                                                                                        cx,
                                                                                    );
                                                                                },
                                                                            );
                                                                        },
                                                                    ),
                                                                );
                                                            }
                                                            menu
                                                        }),
                                                    )
                                                })
                                                .when(row.is_text, |this| {
                                                    this.child(
                                                        gpui::div()
                                                            .max_w(gpui::px(280.0))
                                                            .text_color(theme.muted_foreground)
                                                            .child(row.value),
                                                    )
                                                })
                                        }
                                    })),
                            ),
                    )
                    .child(
                        gpui::div()
                            .h(gpui::px(52.0))
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(gpui::px(8.0))
                            .px(gpui::px(14.0))
                            .border_t(gpui::px(1.0))
                            .border_color(theme.border)
                            .child(
                                Button::new("preferences-reset")
                                    .ghost()
                                    .small()
                                    .label("Reset")
                                    .on_click(move |event, window, cx| {
                                        let _ = reset_editor.update(cx, |editor, cx| {
                                            editor.reset_modal(event, window, cx);
                                        });
                                    }),
                            )
                            .child(gpui::div().flex_1())
                            .child(
                                Button::new("preferences-cancel")
                                    .ghost()
                                    .small()
                                    .label("Cancel")
                                    .on_click(move |event, window, cx| {
                                        let _ = cancel_editor.update(cx, |editor, cx| {
                                            editor.close_modal(event, window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("preferences-done")
                                    .primary()
                                    .small()
                                    .label("Done")
                                    .on_click(move |event, window, cx| {
                                        let _ = done_editor.update(cx, |editor, cx| {
                                            editor.confirm_modal(event, window, cx);
                                        });
                                    }),
                            ),
                    ),
            )
            .into_any_element()
    }
}

struct AetherTitlebarRegion {
    editor: gpui::WeakEntity<AetherEditorView>,
}

impl Render for AetherTitlebarRegion {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let element = self
            .editor
            .update(cx, |editor, editor_cx| {
                editor.render_titlebar_region(editor_cx)
            })
            .unwrap_or_else(|_| gpui::div().into_any_element());
        crate::perf::record_elapsed("frame.gpui_titlebar_region_render", started);
        element
    }
}

struct AetherToolbarRegion {
    editor: gpui::WeakEntity<AetherEditorView>,
}

impl Render for AetherToolbarRegion {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let element = self
            .editor
            .update(cx, |editor, editor_cx| {
                editor.render_toolbar_region(window, editor_cx)
            })
            .unwrap_or_else(|_| gpui::div().into_any_element());
        crate::perf::record_elapsed("frame.gpui_toolbar_region_render", started);
        element
    }
}

struct AetherWorkspaceRegion {
    dock_area: gpui::Entity<gpui_component::dock::DockArea>,
}

struct AetherActivityRailRegion {
    editor: gpui::WeakEntity<AetherEditorView>,
}

impl Render for AetherActivityRailRegion {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let element = self
            .editor
            .update(cx, |editor, editor_cx| {
                editor.render_activity_region(editor_cx)
            })
            .unwrap_or_else(|_| gpui::div().into_any_element());
        crate::perf::record_elapsed("frame.gpui_activity_region_render", started);
        element
    }
}

impl Render for AetherWorkspaceRegion {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let element = gpui::div()
            .id("aether-workspace-dock-area")
            .flex_auto()
            .size_full()
            .min_w(gpui::px(0.0))
            .min_h(gpui::px(0.0))
            .child(self.dock_area.clone());
        crate::perf::record_elapsed("frame.gpui_workspace_region_render", started);
        element
    }
}

struct AetherStatusRegion {
    editor: gpui::WeakEntity<AetherEditorView>,
}

impl Render for AetherStatusRegion {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let element = self
            .editor
            .update(cx, |editor, editor_cx| {
                editor.render_status_region(editor_cx)
            })
            .unwrap_or_else(|_| gpui::div().into_any_element());
        crate::perf::record_elapsed("frame.gpui_status_region_render", started);
        element
    }
}

struct AetherModalRegion {
    editor: gpui::WeakEntity<AetherEditorView>,
}

impl Render for AetherModalRegion {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = std::time::Instant::now();
        let element = self
            .editor
            .update(cx, |editor, editor_cx| {
                editor.render_modal_region(window, editor_cx)
            })
            .unwrap_or_else(|_| gpui::div().into_any_element());
        crate::perf::record_elapsed("frame.gpui_modal_region_render", started);
        element
    }
}

struct HtmlswapTooltipView {
    text: String,
}

impl Render for HtmlswapTooltipView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child(self.text.clone())
    }
}

fn aether_tab_strip(
    strip_id: &'static str,
    tabs: Vec<AetherItem>,
    cx: &mut Context<AetherEditorView>,
) -> gpui::Div {
    let tab_count = tabs.len();
    let divider_color = cx.theme().border.opacity(0.55);
    gpui::div()
        .m(gpui::px(0.0))
        .p(gpui::px(0.0))
        .h_full()
        .flex()
        .items_center()
        .children(tabs.into_iter().enumerate().map(move |(index, tab)| {
            let tab_id = format!("{strip_id}_{}", tab.key);
            let style = tab.style_named("style");
            let icon = tab.icon.clone();
            let label = aether_tab_label(&tab);
            let click_item = tab.clone();
            let is_last = index + 1 == tab_count;
            gpui::div()
                .aether_style(&style)
                .id(tab_id)
                .m(gpui::px(0.0))
                .items_center()
                .gap(gpui::px(6.0))
                .when(!is_last, |this| {
                    this.border_r(gpui::px(1.0))
                        .border_color(divider_color)
                        .mr(gpui::px(1.0))
                })
                .when(!icon.is_empty(), |this| {
                    this.child(
                        gpui::div()
                            .m(gpui::px(0.0))
                            .p(gpui::px(0.0))
                            .block()
                            .flex_none()
                            .text_size(gpui::px(15.0))
                            .child(htmlswap_material_symbol_icon(icon)),
                    )
                })
                .child(gpui::div().m(gpui::px(0.0)).p(gpui::px(0.0)).child(label))
                .on_click(cx.listener(move |this, _event, window, cx| {
                    this.activate_item(&click_item, window, cx);
                    cx.stop_propagation();
                }))
        }))
}

fn aether_tab_label(tab: &AetherItem) -> String {
    if !tab.label.is_empty() {
        tab.label.clone()
    } else if !tab.title.is_empty() {
        tab.title.clone()
    } else {
        tab.name.clone()
    }
}

trait AetherGpuiStyleExt {
    fn aether_style(self, style: &AetherStyle) -> Self;
}

impl AetherGpuiStyleExt for gpui::Div {
    fn aether_style(mut self, style: &AetherStyle) -> Self {
        if style.is_empty() {
            return self;
        }

        if style_eq(style, "display", "none") {
            return self.hidden();
        }

        match normalized(style.get("display")).as_deref() {
            Some("flex" | "inline-flex") => self = self.flex(),
            Some("block" | "inline-block") => self = self.block(),
            _ => {}
        }

        match normalized(style.get("flexDirection")).as_deref() {
            Some("column") => self = self.flex_col(),
            Some("row") => self = self.flex_row(),
            _ => {}
        }

        match normalized(style.get("alignItems")).as_deref() {
            Some("center") => self = self.items_center(),
            Some("flex-start" | "start") => self = self.items_start(),
            Some("flex-end" | "end") => self = self.items_end(),
            _ => {}
        }

        match normalized(style.get("justifyContent")).as_deref() {
            Some("center") => self = self.justify_center(),
            Some("space-between") => self = self.justify_between(),
            Some("flex-start" | "start") => self = self.justify_start(),
            Some("flex-end" | "end") => self = self.justify_end(),
            _ => {}
        }

        if let Some(value) = style.get("width") {
            self = self.w(htmlswap_dynamic_definite_length(value));
        }
        if let Some(value) = style.get("height") {
            self = self.h(htmlswap_dynamic_definite_length(value));
        }
        if let Some(value) = style.get("minWidth") {
            self = self.min_w(htmlswap_dynamic_definite_length(value));
        }
        if let Some(value) = style.get("minHeight") {
            self = self.min_h(htmlswap_dynamic_definite_length(value));
        }
        if let Some(value) = style.get("maxWidth") {
            self = self.max_w(htmlswap_dynamic_definite_length(value));
        }
        if let Some(value) = style.get("maxHeight") {
            self = self.max_h(htmlswap_dynamic_definite_length(value));
        }
        if let Some(value) = style.get("gap") {
            self = self.gap(aether_px(value));
        }

        if let Some(value) = style.get("flex") {
            self = apply_aether_flex(self, value);
        }
        if style_eq(style, "flexGrow", "1") {
            self = self.flex_grow_1();
        }
        if style_eq(style, "flexShrink", "0") {
            self = self.flex_shrink_0();
        }
        if let Some(value) = style.get("flexBasis") {
            self = self.flex_basis(htmlswap_dynamic_length(value));
        }

        self = apply_aether_box_edges(
            self,
            style,
            "padding",
            "paddingTop",
            "paddingRight",
            "paddingBottom",
            "paddingLeft",
        );
        self = apply_aether_box_edges(
            self,
            style,
            "margin",
            "marginTop",
            "marginRight",
            "marginBottom",
            "marginLeft",
        );

        if let Some(value) = style
            .get("background")
            .or_else(|| style.get("backgroundColor"))
        {
            if !is_unsupported_aether_background(value) {
                self = self.bg(htmlswap_dynamic_color(value));
            }
        }
        if let Some(value) = style.get("color") {
            self = self.text_color(htmlswap_dynamic_color(value));
        }
        if let Some(value) = style.get("borderRadius") {
            self = self.rounded(aether_px(value));
        }
        if let Some(value) = style.get("border") {
            self = apply_aether_border(self, value, None);
        }
        if let Some(value) = style.get("borderLeft") {
            self = apply_aether_border(self, value, Some("left"));
        }
        if let Some(value) = style.get("borderRight") {
            self = apply_aether_border(self, value, Some("right"));
        }
        if let Some(value) = style.get("borderTop") {
            self = apply_aether_border(self, value, Some("top"));
        }
        if let Some(value) = style.get("borderBottom") {
            self = apply_aether_border(self, value, Some("bottom"));
        }
        if let Some(value) = style.get("borderColor") {
            self = self.border_color(htmlswap_dynamic_color(value));
        }

        if let Some(value) = style.get("fontSize") {
            self = self.text_size(aether_px(value));
        }
        if let Some(value) = style.get("fontWeight") {
            self = self.font_weight(aether_font_weight(value));
        }
        if let Some(value) = style.get("fontFamily") {
            self = self.font_family(value);
        }

        match normalized(style.get("cursor")).as_deref() {
            Some("default") => self = self.cursor_default(),
            Some("col-resize") => self = self.cursor_col_resize(),
            Some("row-resize") => self = self.cursor_row_resize(),
            Some("pointer") => self = self.cursor(gpui::CursorStyle::PointingHand),
            _ => {}
        }

        match normalized(style.get("overflow")).as_deref() {
            Some("hidden" | "clip") => self = self.overflow_hidden(),
            _ => {}
        }
        match normalized(style.get("overflowX")).as_deref() {
            Some("hidden" | "clip") => self = self.overflow_x_hidden(),
            _ => {}
        }
        match normalized(style.get("overflowY")).as_deref() {
            Some("hidden" | "clip") => self = self.overflow_y_hidden(),
            _ => {}
        }
        if style_eq(style, "textOverflow", "ellipsis") {
            self = self.text_ellipsis();
        }
        if style_eq(style, "whiteSpace", "nowrap") {
            self = self.whitespace_nowrap();
        }

        match normalized(style.get("position")).as_deref() {
            Some("absolute" | "fixed") => self = self.absolute(),
            Some("relative") => self = self.relative(),
            _ => {}
        }
        if let Some(value) = style.get("top") {
            self = self.top(htmlswap_dynamic_length(value));
        }
        if let Some(value) = style.get("right") {
            self = self.right(htmlswap_dynamic_length(value));
        }
        if let Some(value) = style.get("bottom") {
            self = self.bottom(htmlswap_dynamic_length(value));
        }
        if let Some(value) = style.get("left") {
            self = self.left(htmlswap_dynamic_length(value));
        }
        if let Some(value) = style
            .get("opacity")
            .and_then(|value| value.parse::<f32>().ok())
        {
            self = self.opacity(value);
        }

        self
    }
}

fn style_eq(style: &AetherStyle, key: &str, expected: &str) -> bool {
    normalized(style.get(key)).is_some_and(|value| value == expected)
}

fn normalized(value: Option<&str>) -> Option<String> {
    value.map(|value| value.trim().to_ascii_lowercase())
}

fn apply_aether_flex(mut this: gpui::Div, value: &str) -> gpui::Div {
    let value = value.trim();
    if value == "none" {
        return this.flex_none();
    }
    if value == "auto" || value == "1 1 auto" {
        return this.flex_auto();
    }

    let parts = value.split_whitespace().collect::<Vec<_>>();
    if parts.len() == 3 {
        if parts[0] == "0" {
            this = this.flex_shrink_0().flex_none();
        } else if parts[0] == "1" {
            this = this.flex_grow_1();
        }
        this = this.flex_basis(htmlswap_dynamic_length(parts[2]));
    }
    this
}

fn apply_aether_box_edges(
    mut this: gpui::Div,
    style: &AetherStyle,
    shorthand: &str,
    top: &str,
    right: &str,
    bottom: &str,
    left: &str,
) -> gpui::Div {
    if let Some(values) = style.get(shorthand).map(split_aether_box_values) {
        match values.as_slice() {
            [all] => this = apply_box_edge(this, shorthand, all),
            [vertical, horizontal] => {
                this = apply_box_edge(this, top, vertical);
                this = apply_box_edge(this, bottom, vertical);
                this = apply_box_edge(this, right, horizontal);
                this = apply_box_edge(this, left, horizontal);
            }
            [top_v, horizontal, bottom_v] => {
                this = apply_box_edge(this, top, top_v);
                this = apply_box_edge(this, right, horizontal);
                this = apply_box_edge(this, left, horizontal);
                this = apply_box_edge(this, bottom, bottom_v);
            }
            [top_v, right_v, bottom_v, left_v, ..] => {
                this = apply_box_edge(this, top, top_v);
                this = apply_box_edge(this, right, right_v);
                this = apply_box_edge(this, bottom, bottom_v);
                this = apply_box_edge(this, left, left_v);
            }
            _ => {}
        }
    }

    for (key, value) in [
        (top, style.get(top)),
        (right, style.get(right)),
        (bottom, style.get(bottom)),
        (left, style.get(left)),
    ] {
        if let Some(value) = value {
            this = apply_box_edge(this, key, value);
        }
    }

    this
}

fn split_aether_box_values(value: &str) -> Vec<&str> {
    value.split_whitespace().collect()
}

fn apply_box_edge(this: gpui::Div, key: &str, value: &str) -> gpui::Div {
    match key {
        "padding" => this.p(aether_px(value)),
        "paddingTop" => this.pt(aether_px(value)),
        "paddingRight" => this.pr(aether_px(value)),
        "paddingBottom" => this.pb(aether_px(value)),
        "paddingLeft" => this.pl(aether_px(value)),
        "margin" => this.m(aether_px(value)),
        "marginTop" => this.mt(aether_px(value)),
        "marginRight" => this.mr(aether_px(value)),
        "marginBottom" => this.mb(aether_px(value)),
        "marginLeft" => {
            if value.trim().eq_ignore_ascii_case("auto") {
                this.ml(gpui::auto())
            } else {
                this.ml(aether_px(value))
            }
        }
        _ => this,
    }
}

fn apply_aether_border(mut this: gpui::Div, value: &str, edge: Option<&str>) -> gpui::Div {
    let parts = value.split_whitespace().collect::<Vec<_>>();
    let width = parts
        .iter()
        .copied()
        .find(|part| part.ends_with("px") || *part == "0");
    let color = parts.iter().copied().find(|part| {
        part.starts_with('#') || part.starts_with("rgb") || part.eq_ignore_ascii_case("transparent")
    });

    if let Some(width) = width {
        this = match edge {
            Some("left") => this.border_l(aether_px(width)),
            Some("right") => this.border_r(aether_px(width)),
            Some("top") => this.border_t(aether_px(width)),
            Some("bottom") => this.border_b(aether_px(width)),
            _ => this.border(aether_px(width)),
        };
    }
    if let Some(color) = color {
        this = this.border_color(htmlswap_dynamic_color(color));
    }
    this
}

fn aether_px(value: &str) -> gpui::Pixels {
    let value = value.trim();
    let value = value.strip_suffix("px").unwrap_or(value);
    let value = value.strip_prefix('+').unwrap_or(value);
    value
        .parse::<f32>()
        .map(gpui::px)
        .unwrap_or_else(|_| gpui::px(0.0))
}

fn aether_font_weight(value: &str) -> gpui::FontWeight {
    match value.trim() {
        "600" | "700" | "bold" => gpui::FontWeight::BOLD,
        "500" => gpui::FontWeight::MEDIUM,
        _ => gpui::FontWeight::NORMAL,
    }
}

fn is_unsupported_aether_background(value: &str) -> bool {
    value.contains("gradient(")
}

#[derive(Clone)]
struct AetherDockResizeDrag {
    edge: AetherDockResizeEdge,
}

impl Render for AetherDockResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AetherDockResizeEdge {
    Left,
    Right,
    Bottom,
    ScriptTree,
    ScriptOutline,
}

fn aether_left_dock_resize_handle(
    id: &'static str,
    hidden: bool,
    cx: &mut Context<AetherEditorView>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    gpui::div()
        .id(id)
        .m(gpui::px(0.0))
        .p(gpui::px(0.0))
        .w(gpui::px(5.0))
        .flex_shrink_0()
        .flex_none()
        .flex_basis(gpui::px(5.0))
        .when(hidden, |this| this.hidden())
        .cursor_col_resize()
        .bg(theme.background)
        .hover(move |this| this.bg(theme.accent))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, event: &gpui::MouseDownEvent, _window, cx| {
                this.begin_left_dock_resize(event.position);
                cx.stop_propagation();
            }),
        )
        .on_drag(
            AetherDockResizeDrag {
                edge: AetherDockResizeEdge::Left,
            },
            |drag, _offset, _window, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            },
        )
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<AetherDockResizeDrag>, _window, cx| {
                let drag = event.drag(cx);
                if drag.edge == AetherDockResizeEdge::Left {
                    this.resize_left_dock_to(event.event.position, cx);
                }
            },
        ))
}

fn aether_right_dock_resize_handle(
    id: &'static str,
    hidden: bool,
    cx: &mut Context<AetherEditorView>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    gpui::div()
        .id(id)
        .m(gpui::px(0.0))
        .p(gpui::px(0.0))
        .w(gpui::px(5.0))
        .flex_shrink_0()
        .flex_none()
        .flex_basis(gpui::px(5.0))
        .when(hidden, |this| this.hidden())
        .cursor_col_resize()
        .bg(theme.background)
        .hover(move |this| this.bg(theme.accent))
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(move |this, event: &gpui::MouseDownEvent, _window, cx| {
                this.begin_right_dock_resize(event.position);
                cx.stop_propagation();
            }),
        )
        .on_drag(
            AetherDockResizeDrag {
                edge: AetherDockResizeEdge::Right,
            },
            |drag, _offset, _window, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            },
        )
        .on_drag_move(cx.listener(
            move |this, event: &DragMoveEvent<AetherDockResizeDrag>, _window, cx| {
                let drag = event.drag(cx);
                if drag.edge == AetherDockResizeEdge::Right {
                    this.resize_right_dock_to(event.event.position, cx);
                }
            },
        ))
}

fn htmlswap_material_symbol_icon(name: impl AsRef<str>) -> gpui_component::Icon {
    super::dc_icons::material_symbol_icon(name)
}

#[derive(Clone, Copy)]
enum ToolbarAction {
    Save,
    Undo,
    Redo,
    GridSnap,
    AngleSnap,
}

fn dispatch_toolbar_action(action: ToolbarAction, window: &mut Window, cx: &mut gpui::App) {
    match action {
        ToolbarAction::Save => window.dispatch_action(Box::new(az_editor_ui::actions::Save), cx),
        ToolbarAction::Undo => window.dispatch_action(Box::new(az_editor_ui::actions::Undo), cx),
        ToolbarAction::Redo => window.dispatch_action(Box::new(az_editor_ui::actions::Redo), cx),
        ToolbarAction::GridSnap => {
            window.dispatch_action(Box::new(az_editor_ui::actions::ToggleSceneGridSnap), cx);
        }
        ToolbarAction::AngleSnap => {
            window.dispatch_action(Box::new(az_editor_ui::actions::ToggleSceneAngleSnap), cx);
        }
    }
}

fn toolbar_action_button(
    id: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    action: ToolbarAction,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let tooltip_text = tooltip.to_owned();
    gpui::div()
        .id(id)
        .w(gpui::px(28.0))
        .h(gpui::px(28.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(gpui::px(5.0))
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .tooltip(move |_, cx| {
            cx.new(|_| HtmlswapTooltipView {
                text: tooltip_text.clone(),
            })
            .into()
        })
        .on_click(move |_, window, cx| {
            dispatch_toolbar_action(action, window, cx);
            cx.stop_propagation();
        })
        .child(htmlswap_material_symbol_icon(icon))
        .into_any_element()
}

fn scene_snap_chip(
    id: &'static str,
    icon: &'static str,
    label: String,
    active: bool,
    action: ToolbarAction,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let tooltip_text = if matches!(action, ToolbarAction::GridSnap) {
        "Toggle grid snapping"
    } else {
        "Toggle angle snapping"
    };
    gpui::div()
        .id(id)
        .h(gpui::px(28.0))
        .px(gpui::px(8.0))
        .flex()
        .items_center()
        .gap(gpui::px(5.0))
        .rounded(gpui::px(5.0))
        .text_size(gpui::px(10.5))
        .text_color(if active {
            theme.accent
        } else {
            theme.muted_foreground
        })
        .when(active, |this| this.bg(theme.list_active))
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .tooltip(move |_, cx| {
            cx.new(|_| HtmlswapTooltipView {
                text: tooltip_text.to_owned(),
            })
            .into()
        })
        .on_click(move |_, window, cx| {
            dispatch_toolbar_action(action, window, cx);
            cx.stop_propagation();
        })
        .child(htmlswap_material_symbol_icon(icon))
        .child(label)
        .into_any_element()
}

fn activity_rail_button(
    id: &'static str,
    icon: &'static str,
    tooltip: &'static str,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    let tooltip_text = tooltip.to_owned();
    gpui::div()
        .id(id)
        .w(gpui::px(32.0))
        .h(gpui::px(32.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(gpui::px(5.0))
        .text_size(gpui::px(18.0))
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .tooltip(move |_, cx| {
            cx.new(|_| HtmlswapTooltipView {
                text: tooltip_text.clone(),
            })
            .into()
        })
        .on_click(on_click)
        .child(htmlswap_material_symbol_icon(icon))
        .into_any_element()
}

fn aether_empty_state(
    message: impl Into<String>,
    sub: Option<String>,
    cx: &mut Context<AetherEditorView>,
) -> gpui::Div {
    let theme = cx.theme().clone();
    az_editor_ui::panels::kit::empty_state(message, sub, &theme)
}

trait HtmlswapDivTooltipExt {
    fn tooltip<F>(self, _build_tooltip: F) -> Self
    where
        F: Fn(&mut Window, &mut gpui::App) -> gpui::AnyView + 'static;
}

impl HtmlswapDivTooltipExt for gpui::Div {
    fn tooltip<F>(self, _build_tooltip: F) -> Self
    where
        F: Fn(&mut Window, &mut gpui::App) -> gpui::AnyView + 'static,
    {
        self
    }
}

fn htmlswap_dynamic_color(value: impl AsRef<str>) -> gpui::Rgba {
    let value = value.as_ref().trim();
    if let Some(color) = htmlswap_dynamic_hex_color(value) {
        return color;
    }
    if let Some(color) = htmlswap_dynamic_rgb_color(value) {
        return color;
    }

    match value.to_ascii_lowercase().as_str() {
        "transparent" => gpui::rgba(0x00000000),
        "black" => gpui::rgb(0x000000),
        "white" => gpui::rgb(0xFFFFFF),
        "red" => gpui::rgb(0xFF0000),
        "green" => gpui::rgb(0x008000),
        "blue" => gpui::rgb(0x0000FF),
        _ => gpui::rgb(0x000000),
    }
}

fn htmlswap_dynamic_rgb_color(value: &str) -> Option<gpui::Rgba> {
    let value = value.trim();
    let (body, has_alpha) = if let Some(body) = value
        .strip_prefix("rgba(")
        .and_then(|value| value.strip_suffix(')'))
    {
        (body, true)
    } else if let Some(body) = value
        .strip_prefix("rgb(")
        .and_then(|value| value.strip_suffix(')'))
    {
        (body, false)
    } else {
        return None;
    };
    let channels = body.split(',').map(str::trim).collect::<Vec<_>>();
    if (!has_alpha && channels.len() != 3) || (has_alpha && channels.len() != 4) {
        return None;
    }
    let r = channels[0].parse::<u8>().ok()?;
    let g = channels[1].parse::<u8>().ok()?;
    let b = channels[2].parse::<u8>().ok()?;
    if !has_alpha {
        return Some(gpui::rgb(((r as u32) << 16) | ((g as u32) << 8) | b as u32));
    }
    let alpha = parse_aether_alpha(channels[3])?;
    Some(gpui::rgba(
        ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | alpha as u32,
    ))
}

fn parse_aether_alpha(value: &str) -> Option<u8> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let percent = percent.trim().parse::<f32>().ok()?;
        return Some(((percent / 100.0).clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    let alpha = value.parse::<f32>().ok()?;
    Some((alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn htmlswap_dynamic_hex_color(value: &str) -> Option<gpui::Rgba> {
    let hex = value.strip_prefix('#')?;
    match hex.len() {
        3 => {
            let mut expanded = String::with_capacity(6);
            for ch in hex.chars() {
                expanded.push(ch);
                expanded.push(ch);
            }
            u32::from_str_radix(&expanded, 16).ok().map(gpui::rgb)
        }
        4 => {
            let mut expanded = String::with_capacity(8);
            for ch in hex.chars() {
                expanded.push(ch);
                expanded.push(ch);
            }
            u32::from_str_radix(&expanded, 16).ok().map(gpui::rgba)
        }
        6 => u32::from_str_radix(hex, 16).ok().map(gpui::rgb),
        8 => u32::from_str_radix(hex, 16).ok().map(gpui::rgba),
        _ => None,
    }
}

fn htmlswap_dynamic_definite_length(value: impl AsRef<str>) -> gpui::DefiniteLength {
    let value = value.as_ref().trim();
    if let Some(number) = value
        .strip_suffix("px")
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        return gpui::px(number).into();
    }
    if let Some(number) = value
        .strip_suffix("rem")
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        return gpui::rems(number).into();
    }
    if let Some(number) = value
        .strip_suffix("em")
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        return gpui::rems(number).into();
    }
    if let Some(number) = value
        .strip_suffix('%')
        .and_then(|value| value.trim().parse::<f32>().ok())
    {
        return gpui::relative(number / 100.0).into();
    }
    if let Ok(number) = value.parse::<f32>() {
        return gpui::px(number).into();
    }
    gpui::px(0.0).into()
}

fn htmlswap_dynamic_length(value: impl AsRef<str>) -> gpui::Length {
    let value = value.as_ref().trim();
    if value.eq_ignore_ascii_case("auto") || value.eq_ignore_ascii_case("none") {
        return gpui::Length::Auto;
    }
    htmlswap_dynamic_definite_length(value).into()
}

impl AetherEditorView {
    fn render_top_menu_overlay(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let menus = self.menus(_cx);
        let menu_is_open = menus.iter().any(|menu| menu.open);

        gpui::div()
            .id("aether_top_menu_overlay")
            .absolute()
            .top(gpui::px(34.0))
            .left(gpui::px(0.0))
            .right(gpui::px(0.0))
            .bottom(gpui::px(0.0))
            .when(!menu_is_open, |this| this.hidden())
            .on_click(_cx.listener(move |this, _event, _window, _cx| {
                this.close_menu(_event, _window, _cx);
            }))
            .on_mouse_move(
                _cx.listener(move |this, event: &gpui::MouseMoveEvent, _window, _cx| {
                    this.track_menu_pointer(event.position.clone(), _cx);
                }),
            )
            .child(
                gpui::div()
                    .m(gpui::px(0.0))
                    .p(gpui::px(0.0))
                    .pl(gpui::px(8.0))
                    .flex()
                    .items_start()
                    .h(gpui::px(34.0))
                    .child(
                        gpui::div()
                            .w(gpui::px(13.0))
                            .h(gpui::px(0.0))
                            .mr(gpui::px(12.0))
                            .flex_none(),
                    )
                    .children(menus.iter().enumerate().map(|(menu_index, menu)| {
                        let menu = menu.clone();
                        gpui::div()
                            .relative()
                            .h(gpui::px(34.0))
                            .child(
                                gpui::div()
                                    .m(gpui::px(0.0))
                                    .pt(gpui::px(0.0))
                                    .pr(gpui::px(9.0))
                                    .pb(gpui::px(0.0))
                                    .pl(gpui::px(9.0))
                                    .flex()
                                    .items_center()
                                    .h(gpui::px(34.0))
                                    .text_size(gpui::px(12.0))
                                    .opacity(0.0)
                                    .child(format!("{}", menu.name)),
                            )
                            .when(menu.open, |this| {
                                this.child(render_top_menu_panel(_cx, menu_index, &menu))
                            })
                    })),
            )
    }

    fn sync_asset_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let search = self.asset_search();
        if self.asset_search_input.read(cx).value() != search {
            self.asset_search_input.update(cx, |input, cx| {
                input.set_value(search, window, cx);
            });
        }
        let name = self.asset_create_name();
        if self.asset_create_name_input.read(cx).value() != name {
            self.asset_create_name_input.update(cx, |input, cx| {
                input.set_value(name, window, cx);
            });
        }
        let folder = self.asset_create_folder();
        if self.asset_create_folder_input.read(cx).value() != folder {
            self.asset_create_folder_input.update(cx, |input, cx| {
                input.set_value(folder, window, cx);
            });
        }
        let rename_path = self.asset_rename_to_path();
        if self.asset_rename_path_input.read(cx).value() != rename_path {
            self.asset_rename_path_input.update(cx, |input, cx| {
                input.set_value(rename_path, window, cx);
            });
        }
        let add_component_search = self.add_component_search();
        if self.add_component_search_input.read(cx).value() != add_component_search {
            self.add_component_search_input.update(cx, |input, cx| {
                input.set_value(add_component_search, window, cx);
            });
        }
    }

    fn render_asset_create_modal(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = _cx.theme().clone();
        let rows = self.asset_create_rows(_cx);
        let can_submit = self.asset_create_can_submit(_cx);
        let preview_path = self.asset_create_preview_path(_cx);
        let source_root = self.asset_create_source_root_label(_cx);
        let error = self.asset_create_error();

        gpui::div()
            .id("aether_asset_create_modal")
            .w(gpui::px(760.0))
            .max_w(gpui::relative(0.92))
            .max_h(gpui::relative(0.86))
            .bg(theme.background)
            .border(gpui::px(1.0))
            .border_color(theme.border)
            .rounded(gpui::px(8.0))
            .shadow(std::vec![gpui::BoxShadow {
                color: theme.border.opacity(0.60).into(),
                offset: gpui::point(gpui::px(0.0), gpui::px(28.0)),
                blur_radius: gpui::px(70.0),
                spread_radius: gpui::px(0.0),
                inset: false,
            }])
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_click(_cx.listener(move |this, _event, _window, _cx| {
                this.stop_prop(_event, _window, _cx);
            }))
            .child(
                gpui::div()
                    .h(gpui::px(52.0))
                    .px(gpui::px(18.0))
                    .flex()
                    .items_center()
                    .gap(gpui::px(10.0))
                    .border_b(gpui::px(1.0))
                    .border_color(theme.border)
                    .bg(theme.tab_bar)
                    .child(
                        gpui::div()
                            .text_size(gpui::px(20.0))
                            .text_color(theme.accent)
                            .child(htmlswap_material_symbol_icon("add")),
                    )
                    .child(
                        gpui::div()
                            .flex_1()
                            .child(
                                gpui::div()
                                    .text_size(gpui::px(14.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .child("New Asset"),
                            )
                            .child(
                                gpui::div()
                                    .text_size(gpui::px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child("Create a source file through the asset processor"),
                            ),
                    )
                    .child(
                        gpui::div()
                            .id("aether_asset_create_close")
                            .w(gpui::px(30.0))
                            .h(gpui::px(30.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(gpui::px(6.0))
                            .text_color(theme.muted_foreground)
                            .hover(|this| this.bg(theme.muted).text_color(theme.foreground))
                            .cursor_default()
                            .on_click(_cx.listener(move |this, _event, _window, _cx| {
                                this.close_modal(_event, _window, _cx);
                            }))
                            .child(htmlswap_material_symbol_icon("close")),
                    ),
            )
            .child(
                gpui::div()
                    .flex()
                    .min_h(gpui::px(0.0))
                    .child(
                        gpui::div()
                            .id("aether_asset_create_source_list")
                            .w(gpui::px(300.0))
                            .flex_none()
                            .p(gpui::px(12.0))
                            .border_r(gpui::px(1.0))
                            .border_color(theme.border)
                            .overflow_y_scroll()
                            .children(rows.into_iter().enumerate().map(|(index, row)| {
                                if row.sep {
                                    return gpui::div()
                                        .mt(gpui::px(if index == 0 { 0.0 } else { 10.0 }))
                                        .mb(gpui::px(5.0))
                                        .text_size(gpui::px(10.0))
                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                        .text_color(theme.muted_foreground)
                                        .child(row.label)
                                        .into_any_element();
                                }
                                let schema_type = row.key.clone();
                                gpui::div()
                                    .aether_style(&row.style_named("style"))
                                    .id(format!("aether_asset_create_source_{index}"))
                                    .mb(gpui::px(6.0))
                                    .hover(|this| this.bg(theme.muted))
                                    .on_click(_cx.listener(move |this, _event, _window, _cx| {
                                        this.select_asset_create_schema(&schema_type, _cx);
                                        _cx.stop_propagation();
                                    }))
                                    .child(
                                        gpui::div()
                                            .flex()
                                            .items_center()
                                            .justify_between()
                                            .gap(gpui::px(8.0))
                                            .child(
                                                gpui::div()
                                                    .text_size(gpui::px(12.0))
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .text_color(theme.foreground)
                                                    .child(row.label),
                                            )
                                            .child(
                                                gpui::div()
                                                    .font_family("IBM Plex Mono")
                                                    .text_size(gpui::px(10.0))
                                                    .text_color(theme.muted_foreground)
                                                    .child(row.ext),
                                            ),
                                    )
                                    .child(
                                        gpui::div()
                                            .mt(gpui::px(3.0))
                                            .font_family("IBM Plex Mono")
                                            .text_size(gpui::px(10.0))
                                            .text_color(theme.muted_foreground)
                                            .child(row.key),
                                    )
                                    .into_any_element()
                            })),
                    )
                    .child(
                        gpui::div()
                            .flex_1()
                            .min_w(gpui::px(0.0))
                            .p(gpui::px(16.0))
                            .flex()
                            .flex_col()
                            .gap(gpui::px(12.0))
                            .child(asset_create_labeled_input(
                                "Name",
                                &self.asset_create_name_input,
                                "aether_asset_create_name",
                                &theme,
                            ))
                            .child(asset_create_labeled_input(
                                "Target folder",
                                &self.asset_create_folder_input,
                                "aether_asset_create_folder",
                                &theme,
                            ))
                            .child(
                                gpui::div()
                                    .rounded(gpui::px(6.0))
                                    .border(gpui::px(1.0))
                                    .border_color(theme.border)
                                    .bg(theme.secondary)
                                    .p(gpui::px(10.0))
                                    .font_family("IBM Plex Mono")
                                    .text_size(gpui::px(11.0))
                                    .text_color(theme.foreground)
                                    .child(preview_path),
                            )
                            .child(
                                gpui::div()
                                    .font_family("IBM Plex Mono")
                                    .text_size(gpui::px(10.0))
                                    .text_color(theme.muted_foreground)
                                    .child(source_root),
                            )
                            .when(!error.is_empty(), |this| {
                                this.child(
                                    gpui::div()
                                        .rounded(gpui::px(6.0))
                                        .border(gpui::px(1.0))
                                        .border_color(theme.danger)
                                        .bg(theme.danger.opacity(0.12))
                                        .p(gpui::px(10.0))
                                        .text_size(gpui::px(11.0))
                                        .text_color(theme.danger_foreground)
                                        .child(error),
                                )
                            })
                            .child(gpui::div().flex_1())
                            .child(
                                gpui::div()
                                    .flex()
                                    .justify_end()
                                    .gap(gpui::px(8.0))
                                    .child(
                                        gpui::div()
                                            .id("aether_asset_create_cancel")
                                            .px(gpui::px(12.0))
                                            .h(gpui::px(30.0))
                                            .flex()
                                            .items_center()
                                            .rounded(gpui::px(6.0))
                                            .border(gpui::px(1.0))
                                            .border_color(theme.border)
                                            .text_size(gpui::px(12.0))
                                            .text_color(theme.foreground)
                                            .hover(|this| this.bg(theme.muted))
                                            .cursor_default()
                                            .on_click(_cx.listener(
                                                move |this, _event, _window, _cx| {
                                                    this.close_modal(_event, _window, _cx);
                                                },
                                            ))
                                            .child("Cancel"),
                                    )
                                    .child(
                                        gpui::div()
                                            .id("aether_asset_create_submit")
                                            .px(gpui::px(14.0))
                                            .h(gpui::px(30.0))
                                            .flex()
                                            .items_center()
                                            .rounded(gpui::px(6.0))
                                            .bg(if can_submit {
                                                theme.primary
                                            } else {
                                                theme.muted
                                            })
                                            .text_size(gpui::px(12.0))
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .text_color(if can_submit {
                                                theme.primary_foreground
                                            } else {
                                                theme.muted_foreground
                                            })
                                            .when(can_submit, |this| {
                                                this.hover(|this| this.bg(theme.primary_hover))
                                            })
                                            .cursor_default()
                                            .on_click(_cx.listener(Self::submit_asset_create))
                                            .child("Create"),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_asset_rename_modal(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = _cx.theme().clone();
        let from_path = self.asset_rename_from_path();
        let source_root = self.asset_rename_source_root_label();
        let error = self.asset_rename_error();
        let can_submit = self.asset_rename_can_submit();

        gpui::div()
            .id("aether_asset_rename_modal")
            .w(gpui::px(540.0))
            .max_w(gpui::relative(0.92))
            .bg(theme.background)
            .border(gpui::px(1.0))
            .border_color(theme.border)
            .rounded(gpui::px(8.0))
            .shadow(std::vec![gpui::BoxShadow {
                color: theme.border.opacity(0.60).into(),
                offset: gpui::point(gpui::px(0.0), gpui::px(28.0)),
                blur_radius: gpui::px(70.0),
                spread_radius: gpui::px(0.0),
                inset: false,
            }])
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_click(_cx.listener(move |this, _event, _window, _cx| {
                this.stop_prop(_event, _window, _cx);
            }))
            .child(asset_crud_modal_header(
                "drive_file_rename_outline",
                "Rename Asset",
                "Move or rename the source path without changing its asset identity",
                &theme,
                _cx,
            ))
            .child(
                gpui::div()
                    .p(gpui::px(16.0))
                    .flex()
                    .flex_col()
                    .gap(gpui::px(12.0))
                    .child(
                        gpui::div()
                            .rounded(gpui::px(6.0))
                            .border(gpui::px(1.0))
                            .border_color(theme.border)
                            .bg(theme.secondary)
                            .p(gpui::px(10.0))
                            .font_family("IBM Plex Mono")
                            .text_size(gpui::px(11.0))
                            .text_color(theme.muted_foreground)
                            .child(from_path),
                    )
                    .child(asset_create_labeled_input(
                        "New source path",
                        &self.asset_rename_path_input,
                        "aether_asset_rename_path",
                        &theme,
                    ))
                    .child(
                        gpui::div()
                            .font_family("IBM Plex Mono")
                            .text_size(gpui::px(10.0))
                            .text_color(theme.muted_foreground)
                            .child(source_root),
                    )
                    .when(!error.is_empty(), |this| {
                        this.child(asset_crud_error(&error, &theme))
                    })
                    .child(
                        gpui::div()
                            .flex()
                            .justify_end()
                            .gap(gpui::px(8.0))
                            .child(
                                gpui::div()
                                    .id("aether_asset_rename_cancel")
                                    .px(gpui::px(12.0))
                                    .h(gpui::px(30.0))
                                    .flex()
                                    .items_center()
                                    .rounded(gpui::px(6.0))
                                    .border(gpui::px(1.0))
                                    .border_color(theme.border)
                                    .text_size(gpui::px(12.0))
                                    .text_color(theme.foreground)
                                    .hover(|this| this.bg(theme.muted))
                                    .cursor_pointer()
                                    .on_click(_cx.listener(move |this, _event, _window, _cx| {
                                        this.close_modal(_event, _window, _cx);
                                    }))
                                    .child("Cancel"),
                            )
                            .child(
                                gpui::div()
                                    .id("aether_asset_rename_submit")
                                    .px(gpui::px(14.0))
                                    .h(gpui::px(30.0))
                                    .flex()
                                    .items_center()
                                    .rounded(gpui::px(6.0))
                                    .bg(if can_submit {
                                        theme.primary
                                    } else {
                                        theme.muted
                                    })
                                    .text_size(gpui::px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if can_submit {
                                        theme.primary_foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .when(can_submit, |this| {
                                        this.hover(|this| this.bg(theme.primary_hover))
                                    })
                                    .cursor_default()
                                    .on_click(_cx.listener(Self::submit_asset_rename))
                                    .child("Rename"),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_asset_delete_modal(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let theme = _cx.theme().clone();
        let source_path = self.asset_delete_source_path();
        let source_root = self.asset_delete_source_root_label();
        let summary = self.asset_delete_summary(_cx);
        let rows = self.asset_delete_dependent_rows(_cx);
        let error = self.asset_delete_error(_cx);
        let can_submit = self.asset_delete_can_submit(_cx);

        gpui::div()
            .id("aether_asset_delete_modal")
            .w(gpui::px(600.0))
            .max_w(gpui::relative(0.92))
            .max_h(gpui::relative(0.80))
            .bg(theme.background)
            .border(gpui::px(1.0))
            .border_color(theme.danger)
            .rounded(gpui::px(8.0))
            .shadow(std::vec![gpui::BoxShadow {
                color: theme.danger.opacity(0.25).into(),
                offset: gpui::point(gpui::px(0.0), gpui::px(28.0)),
                blur_radius: gpui::px(70.0),
                spread_radius: gpui::px(0.0),
                inset: false,
            }])
            .flex()
            .flex_col()
            .overflow_hidden()
            .on_click(_cx.listener(move |this, _event, _window, _cx| {
                this.stop_prop(_event, _window, _cx);
            }))
            .child(asset_crud_modal_header(
                "delete",
                "Delete Asset",
                "Remove the source file and retire current products",
                &theme,
                _cx,
            ))
            .child(
                gpui::div()
                    .p(gpui::px(16.0))
                    .flex()
                    .flex_col()
                    .gap(gpui::px(12.0))
                    .child(
                        gpui::div()
                            .rounded(gpui::px(6.0))
                            .border(gpui::px(1.0))
                            .border_color(theme.border)
                            .bg(theme.secondary)
                            .p(gpui::px(10.0))
                            .font_family("IBM Plex Mono")
                            .text_size(gpui::px(11.0))
                            .text_color(theme.foreground)
                            .child(source_path),
                    )
                    .child(
                        gpui::div()
                            .font_family("IBM Plex Mono")
                            .text_size(gpui::px(10.0))
                            .text_color(theme.muted_foreground)
                            .child(source_root),
                    )
                    .child(
                        gpui::div()
                            .text_size(gpui::px(12.0))
                            .text_color(theme.foreground)
                            .child(summary),
                    )
                    .when(!rows.is_empty(), |this| {
                        this.child(
                            gpui::div()
                                .max_h(gpui::px(180.0))
                                .overflow_hidden()
                                .border(gpui::px(1.0))
                                .border_color(theme.border)
                                .rounded(gpui::px(6.0))
                                .children(rows.into_iter().map(|row| {
                                    gpui::div()
                                        .px(gpui::px(10.0))
                                        .py(gpui::px(8.0))
                                        .border_b(gpui::px(1.0))
                                        .border_color(theme.border)
                                        .flex()
                                        .items_center()
                                        .gap(gpui::px(8.0))
                                        .child(
                                            gpui::div()
                                                .text_size(gpui::px(16.0))
                                                .text_color(htmlswap_dynamic_color(row.color))
                                                .child(htmlswap_material_symbol_icon(row.icon)),
                                        )
                                        .child(
                                            gpui::div()
                                                .flex_1()
                                                .min_w(gpui::px(0.0))
                                                .child(
                                                    gpui::div()
                                                        .font_family("IBM Plex Mono")
                                                        .text_size(gpui::px(11.0))
                                                        .text_color(theme.foreground)
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .child(row.name),
                                                )
                                                .child(
                                                    gpui::div()
                                                        .mt(gpui::px(2.0))
                                                        .text_size(gpui::px(10.0))
                                                        .text_color(theme.muted_foreground)
                                                        .overflow_hidden()
                                                        .text_ellipsis()
                                                        .child(row.meta),
                                                ),
                                        )
                                })),
                        )
                    })
                    .when(!error.is_empty(), |this| {
                        this.child(asset_crud_error(&error, &theme))
                    })
                    .child(
                        gpui::div()
                            .flex()
                            .justify_end()
                            .gap(gpui::px(8.0))
                            .child(
                                gpui::div()
                                    .id("aether_asset_delete_cancel")
                                    .px(gpui::px(12.0))
                                    .h(gpui::px(30.0))
                                    .flex()
                                    .items_center()
                                    .rounded(gpui::px(6.0))
                                    .border(gpui::px(1.0))
                                    .border_color(theme.border)
                                    .text_size(gpui::px(12.0))
                                    .text_color(theme.foreground)
                                    .hover(|this| this.bg(theme.muted))
                                    .cursor_default()
                                    .on_click(_cx.listener(move |this, _event, _window, _cx| {
                                        this.close_modal(_event, _window, _cx);
                                    }))
                                    .child("Cancel"),
                            )
                            .child(
                                gpui::div()
                                    .id("aether_asset_delete_submit")
                                    .px(gpui::px(14.0))
                                    .h(gpui::px(30.0))
                                    .flex()
                                    .items_center()
                                    .rounded(gpui::px(6.0))
                                    .bg(if can_submit {
                                        theme.danger
                                    } else {
                                        theme.muted
                                    })
                                    .text_size(gpui::px(12.0))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(if can_submit {
                                        theme.danger_foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .when(can_submit, |this| {
                                        this.hover(|this| this.bg(theme.danger.opacity(0.84)))
                                    })
                                    .cursor_default()
                                    .on_click(_cx.listener(Self::submit_asset_delete))
                                    .child("Delete"),
                            ),
                    ),
            )
            .into_any_element()
    }
}

fn asset_create_labeled_input(
    label: &'static str,
    input: &gpui::Entity<gpui_component::input::InputState>,
    id: &'static str,
    theme: &gpui_component::theme::Theme,
) -> gpui::AnyElement {
    gpui::div()
        .flex()
        .flex_col()
        .gap(gpui::px(5.0))
        .child(
            gpui::div()
                .text_size(gpui::px(11.0))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            gpui::div()
                .id(id)
                .h(gpui::px(30.0))
                .px(gpui::px(9.0))
                .flex()
                .items_center()
                .rounded(gpui::px(6.0))
                .border(gpui::px(1.0))
                .border_color(theme.border)
                .bg(theme.input)
                .child(
                    gpui_component::input::Input::new(input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                ),
        )
        .into_any_element()
}

fn asset_crud_modal_header(
    icon: &'static str,
    title: &'static str,
    subtitle: &'static str,
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherEditorView>,
) -> gpui::AnyElement {
    gpui::div()
        .h(gpui::px(52.0))
        .px(gpui::px(18.0))
        .flex()
        .items_center()
        .gap(gpui::px(10.0))
        .border_b(gpui::px(1.0))
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .child(
            gpui::div()
                .text_size(gpui::px(20.0))
                .text_color(theme.accent)
                .child(htmlswap_material_symbol_icon(icon)),
        )
        .child(
            gpui::div()
                .flex_1()
                .child(
                    gpui::div()
                        .text_size(gpui::px(14.0))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child(title),
                )
                .child(
                    gpui::div()
                        .text_size(gpui::px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(subtitle),
                ),
        )
        .child(
            gpui::div()
                .id(format!("aether_asset_crud_close_{title}"))
                .w(gpui::px(30.0))
                .h(gpui::px(30.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(gpui::px(6.0))
                .text_color(theme.muted_foreground)
                .hover(|this| this.bg(theme.muted).text_color(theme.foreground))
                .cursor_default()
                .on_click(cx.listener(move |this, _event, _window, _cx| {
                    this.close_modal(_event, _window, _cx);
                }))
                .child(htmlswap_material_symbol_icon("close")),
        )
        .into_any_element()
}

fn asset_crud_error(error: &str, theme: &gpui_component::theme::Theme) -> gpui::AnyElement {
    gpui::div()
        .rounded(gpui::px(6.0))
        .border(gpui::px(1.0))
        .border_color(theme.danger)
        .bg(theme.danger.opacity(0.12))
        .p(gpui::px(10.0))
        .text_size(gpui::px(11.0))
        .text_color(theme.danger_foreground)
        .child(error.to_owned())
        .into_any_element()
}

fn asset_source_context_menu(
    mut popup: PopupMenu,
    view: gpui::WeakEntity<AetherEditorView>,
    item: AetherItem,
) -> PopupMenu {
    let rename_view = view.clone();
    let rename_item = item.clone();
    popup = popup.item(
        PopupMenuItem::new("Rename")
            .icon(htmlswap_material_symbol_icon("drive_file_rename_outline"))
            .on_click(move |_, _window, cx| {
                let _ = rename_view.update(cx, |this, cx| {
                    this.open_asset_rename_modal(&rename_item, cx);
                });
            }),
    );

    let delete_view = view;
    popup = popup.item(PopupMenuItem::separator());
    popup = popup.item(
        PopupMenuItem::new("Delete")
            .icon(htmlswap_material_symbol_icon("delete"))
            .on_click(move |_, window, cx| {
                let _ = delete_view.update(cx, |this, cx| {
                    this.open_asset_delete_modal(&item, window, cx);
                });
            }),
    );

    popup.min_w(gpui::px(168.0))
}

fn render_top_menu_panel(
    _cx: &mut Context<AetherEditorView>,
    menu_index: usize,
    menu: &AetherItem,
) -> impl IntoElement {
    let theme = _cx.theme().clone();
    gpui::div()
        .id(format!("aether_top_menu_panel_{menu_index}"))
        .m(gpui::px(0.0))
        .p(gpui::px(5.0))
        .absolute()
        .top(gpui::px(0.0))
        .left(gpui::px(0.0))
        .min_w(gpui::px(228.0))
        .bg(theme.popover)
        .border(gpui::px(1.0))
        .border_color(theme.border)
        .rounded(gpui::px(6.0))
        .shadow(std::vec![gpui::BoxShadow {
            color: theme.background.opacity(0.5).into(),
            offset: gpui::point(gpui::px(0.0), gpui::px(12.0)),
            blur_radius: gpui::px(30.0),
            spread_radius: gpui::px(0.0),
            inset: false,
        }])
        .on_mouse_down(gpui::MouseButton::Left, |_, _, _cx| {
            _cx.stop_propagation();
        })
        .children(
            menu.items
                .iter()
                .enumerate()
                .map(move |(item_index, item)| {
                    let item = item.clone();
                    gpui::div()
                        .when(item.sep, |this| {
                            this.child(
                                gpui::div()
                                    .mt(gpui::px(5.0))
                                    .mr(gpui::px(6.0))
                                    .mb(gpui::px(5.0))
                                    .ml(gpui::px(6.0))
                                    .h(gpui::px(1.0))
                                    .bg(theme.border),
                            )
                        })
                        .when(item.notsep, |this| {
                            this.child(
                                gpui::div()
                                    .aether_style(&item.style_named("style"))
                                    .m(gpui::px(0.0))
                                    .pt(gpui::px(0.0))
                                    .pr(gpui::px(9.0))
                                    .pb(gpui::px(0.0))
                                    .pl(gpui::px(9.0))
                                    .id(format!("aether_top_menu_item_{menu_index}_{item_index}"))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .h(gpui::px(27.0))
                                    .rounded(gpui::px(4.0))
                                    .text_size(gpui::px(11.5))
                                    .text_color(theme.popover_foreground)
                                    .when(item.disabled, |this| this.opacity(0.45))
                                    .cursor_default()
                                    .when(!item.disabled, |this| {
                                        this.hover(|this| {
                                            this.bg(theme.accent)
                                                .text_color(theme.accent_foreground)
                                        })
                                    })
                                    .on_mouse_down(gpui::MouseButton::Left, {
                                        let item = item.clone();
                                        _cx.listener(
                                            move |this,
                                                  _event: &gpui::MouseDownEvent,
                                                  _window,
                                                  _cx| {
                                                this.activate_menu_item(&item, _window, _cx);
                                                _cx.stop_propagation();
                                            },
                                        )
                                    })
                                    .on_click(_cx.listener(move |_this, _event, _window, _cx| {
                                        _cx.stop_propagation();
                                    }))
                                    .child(
                                        gpui::div()
                                            .flex()
                                            .items_center()
                                            .gap(gpui::px(8.0))
                                            .child(
                                                gpui::div()
                                                    .block()
                                                    .text_size(gpui::px(16.0))
                                                    .w(gpui::px(16.0))
                                                    .text_color(theme.muted_foreground)
                                                    .child(htmlswap_material_symbol_icon(format!(
                                                        "{}",
                                                        item.icon
                                                    ))),
                                            )
                                            .child(format!("{}", item.label)),
                                    )
                                    .child(
                                        gpui::div()
                                            .font_family(theme.mono_font_family.clone())
                                            .text_size(gpui::px(10.0))
                                            .text_color(theme.muted_foreground)
                                            .child(format!("{}", item.shortcut)),
                                    ),
                            )
                        })
                }),
        )
}

fn render_top_menu_dropdown(
    cx: &mut Context<AetherEditorView>,
    menu_index: usize,
    menu: &AetherItem,
) -> gpui::AnyElement {
    let view = cx.entity().downgrade();
    let items = menu.items.clone();

    Button::new(format!("aether_top_menu_button_{menu_index}"))
        .small()
        .compact()
        .ghost()
        .label(menu.name.clone())
        .h(gpui::px(34.0))
        .px_2()
        .text_size(gpui::px(12.0))
        .dropdown_menu(move |mut popup, _window, _cx| {
            for item in items.iter().cloned() {
                if item.sep {
                    popup = popup.item(PopupMenuItem::separator());
                    continue;
                }

                let label = if item.label.is_empty() {
                    item.name.clone()
                } else {
                    item.label.clone()
                };
                let menu_item = PopupMenuItem::new(label)
                    .disabled(item.disabled)
                    .when(!item.icon.is_empty(), |this| {
                        this.icon(htmlswap_material_symbol_icon(&item.icon))
                    })
                    .on_click({
                        let view = view.clone();
                        let item = item.clone();
                        move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.activate_menu_item(&item, window, cx);
                            });
                        }
                    });

                popup = popup.item(menu_item);
            }

            popup
                .min_w(gpui::px(228.0))
                .max_h(gpui::px(420.0))
                .scrollable(true)
        })
        .into_any_element()
}

fn render_mode_toolbar_strip(
    cx: &mut Context<AetherEditorView>,
    id_prefix: &'static str,
    breadcrumb: Option<String>,
    items: Vec<AetherItem>,
) -> gpui::AnyElement {
    let theme = cx.theme().clone();
    let view = cx.entity().downgrade();
    gpui::div()
        .flex()
        .items_center()
        .gap(gpui::px(2.0))
        .children(breadcrumb.map(|label| {
            gpui::div()
                .flex()
                .items_center()
                .gap(gpui::px(6.0))
                .h(gpui::px(28.0))
                .px(gpui::px(11.0))
                .rounded(gpui::px(6.0))
                .text_size(gpui::px(11.0))
                .text_color(theme.info)
                .child(
                    gpui::div()
                        .block()
                        .text_size(gpui::px(16.0))
                        .child(htmlswap_material_symbol_icon("account_tree")),
                )
                .child(label)
        }))
        .children(items.into_iter().enumerate().map(|(index, item)| {
            let view = view.clone();
            let disabled = item.kind == "disabled";
            let tooltip = item.title.clone();
            let item_for_click = item.clone();
            gpui::div()
                .id(format!("aether-{id_prefix}-tool-{index}"))
                .flex()
                .items_center()
                .gap(gpui::px(6.0))
                .h(gpui::px(28.0))
                .px(gpui::px(11.0))
                .rounded(gpui::px(6.0))
                .text_size(gpui::px(11.0))
                .text_color(theme.muted_foreground)
                .tooltip(move |_, cx| {
                    cx.new(|_| HtmlswapTooltipView {
                        text: tooltip.clone(),
                    })
                    .into()
                })
                .when(disabled, |this| this.opacity(0.45).cursor_default())
                .when(!disabled, |this| {
                    this.cursor_pointer()
                        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.activate_toolbar_item(&item_for_click, window, cx);
                            });
                            cx.stop_propagation();
                        })
                })
                .child(
                    gpui::div()
                        .block()
                        .text_size(gpui::px(17.0))
                        .child(htmlswap_material_symbol_icon(item.icon.clone())),
                )
                .child(item.label.clone())
        }))
        .into_any_element()
}

impl Render for AetherEditorView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let perf_render_started = std::time::Instant::now();
        self.cleanup_authored_source_path_moves(_cx);
        self.sync_asset_inputs(_window, _cx);
        // html-comment: ================= MENU / TITLE BAR =================
        // html-comment: ===== LEVEL SWITCHER DROPDOWN (anchored to titlebar so it escapes the breadcrumb clip) =====
        // html-comment: ================= TOOLBAR =================
        // html-comment: left tools (shrink + clip gracefully when space is tight)
        // html-comment: /left tools
        // html-comment: play controls (always centered between equal side sections)
        // html-comment: ================= WORKSPACE =================
        // html-comment: activity rail
        // html-comment: ============ SCENE MODE (default workspace) ============
        // html-comment: MAIN AREA: upper docks row, then full-width bottom dock
        // html-comment: LEFT DOCK : hierarchy
        // html-comment: CENTER COLUMN
        // html-comment: viewport tabs
        // html-comment: viewport
        // html-comment: three.js viewport
        // html-comment: top-left view option pills (functional dropdowns)
        // html-comment: top-right stats
        // html-comment: simulating badge
        // html-comment: bottom-left nav info
        // html-comment: bottom-right orientation triad
        // html-comment: /upper docks row
        // html-comment: BOTTOM DOCK (full width, spans under left+center+right)
        // html-comment: BOTTOM DOCK
        // html-comment: ASSET BROWSER
        // html-comment: CONSOLE
        // html-comment: OUTPUT LOG
        // html-comment: PROFILER
        // html-comment: GEMS / PLUGINS
        // html-comment: /main area
        // html-comment: RIGHT DOCK : inspector
        // html-comment: entity header
        // html-comment: components
        // html-comment: PREFAB view
        // html-comment: prefab source banner
        // html-comment: instance actions
        // html-comment: overrides
        // html-comment: nested structure
        // html-comment: /scene mode
        // html-comment: ============ MATERIAL EDITOR MODE ============
        // html-comment: mat toolbar
        // html-comment: node palette
        // html-comment: node graph canvas
        // html-comment: node graph canvas (React Flow)
        // html-comment: preview + params
        // html-comment: /material editor
        // html-comment: ============ SCRIPT EDITOR MODE ============
        // html-comment: file tree
        // html-comment: code
        // html-comment: outline
        // html-comment: /script editor
        // html-comment: ============ GAME DATA MODE ============
        // html-comment: LEFT : tables & schemas
        // html-comment: view toggle
        // html-comment: TABLES grouped by UI category folders
        // html-comment: SCHEMAS
        // html-comment: MANAGERS grouped by owner gem
        // html-comment: CENTER
        // html-comment: ===== DATA GRID (table selected) =====
        // html-comment: toolbar
        // html-comment: grid
        // html-comment: header
        // html-comment: rows
        // html-comment: add-row ghost
        // html-comment: footer
        // html-comment: /data grid
        // html-comment: ===== SCHEMA EDITOR (schema selected) =====
        // html-comment: /schema editor
        // html-comment: ===== MANAGER COMPOSITION (manager selected) =====
        // html-comment: header
        // html-comment: body
        // html-comment: pipeline
        // html-comment: two columns: sources+projection / descriptor
        // html-comment: sources
        // html-comment: projection
        // html-comment: consumers
        // html-comment: descriptor
        // html-comment: /manager composition
        // html-comment: RIGHT : schema inspector
        // html-comment: FIELD INSPECTOR
        // html-comment: reference target (foreign key)
        // html-comment: TABLE INSPECTOR
        // html-comment: SCHEMA INSPECTOR
        // html-comment: MANAGER INSPECTOR
        // html-comment: READ-ONLY panel for auto-generated managers
        // html-comment: EDITABLE controls for hand-authored managers
        // html-comment: /game data
        // html-comment: ============ SEQUENCER MODE ============
        // html-comment: transport bar
        // html-comment: time ruler
        // html-comment: tracks + lanes (shared vertical scroll) with playhead overlay
        // html-comment: footer
        // html-comment: /sequencer
        // html-comment: ============ ANIMATION EDITOR MODE ============
        // html-comment: LEFT DOCK : browsers + parameters
        // html-comment: Anim Graph nav (state list)
        // html-comment: Motions
        // html-comment: Motion Sets
        // html-comment: Skeleton
        // html-comment: Parameters panel (always visible)
        // html-comment: CENTER : anim graph canvas + time view
        // html-comment: breadcrumb bar
        // html-comment: graph canvas
        // html-comment: bottom : motion event Time View
        // html-comment: RIGHT DOCK : mannequin viewport + attributes
        // html-comment: /animation
        // html-comment: ================= STATUS BAR =================
        // html-comment: left panel toggle (mapped to the left dock it controls)
        // html-comment: shrinkable info cluster (clips before it can leak)
        // html-comment: compiler / build status
        // html-comment: asset pipeline
        // html-comment: source control
        // html-comment: selection + file type
        // html-comment: diagnostics
        // html-comment: /info cluster
        // html-comment: ============ RIGHT CLUSTER ============
        // html-comment: panel show/hide toggles — pinned to the far right of the status bar
        // html-comment: pipeline popover
        // html-comment: ================= MODALS (Preferences / Project Settings / About) =================
        // html-comment: ================= NEW MANAGER WIZARD =================
        // html-comment: header
        // html-comment: step rail
        // html-comment: body
        // html-comment: STEP 1: IDENTITY
        // html-comment: STEP 2: SOURCES
        // html-comment: STEP 3: POLICIES
        // html-comment: STEP 4: REVIEW
        // html-comment: footer
        // html-comment: select dropdown popover (fixed, escapes the modal scroll area)
        // html-comment: menu outside-click catcher
        // js-line-comment: ---- scene layers ----
        // js-line-comment: ---- script editor ----
        // js-line-comment: ---- build dropdown ----
        // js-line-comment: ---- modal: editor Preferences + Project Settings ----
        // js-line-comment: row types: head | toggle | seg | select | slider | color | keybind | text
        // js-line-comment: ===================== GAME DATA =====================
        // js-line-comment: Categories are purely a UI-level organization (folders) — independent of structure.
        // js-line-comment: Schemas define a row TYPE (fields + types). Many tables can share one schema.
        // js-line-comment: Tables are datasets that conform to a schema. Note three tables share sch_damage.
        // js-line-comment: ===================== DATA MANAGERS =====================
        // js-line-comment: Managers are the access layer: game logic asks a manager for a typed view,
        // js-line-comment: instead of knowing which table / row / column the data lives in. A manager
        // js-line-comment: can pull from GameData tables, table families, prefabs/product assets, or
        // js-line-comment: other managers — or any mix of them.
        // js-line-comment: Owners: a manager descriptor is source that lives in a project gem / authoring crate.
        // js-line-comment: sources: {kind:'table',ref} | {kind:'family',schema} | {kind:'manager',ref} | {kind:'asset',assetType,label}
        // js-line-comment: Single-backed managers (one table, keyed) are auto-generated per table — see autoManagers().
        // js-line-comment: Hand-authored managers below all join multiple sources or reshape rows.
        // js-line-comment: ===================== SEQUENCER =====================
        // js-line-comment: ===================== ANIMATION EDITOR (EmotionFX) =====================
        // js-line-comment: Anim graph — a root state machine. Locomotion is a blend tree (blend space 2D).
        // js-line-comment: Jump sub-state-machine (drill in by double-clicking the Jump node)
        // js-line-comment: ---- levels & streaming sublevels ----
        // js-line-comment: ===================== THREE.JS VIEWPORT =====================
        // js-line-comment: ---- CodeMirror 6 (real code editor) ----
        // js-line-comment: ---- React Flow (real node graph) ----
        // js-line-comment: ---- custom material node: header + optional texture thumbnail + typed sockets ----
        // js-line-comment: forces React Flow to read each node's real handle positions so edges can route socket-to-socket
        // js-line-comment: ---- mode-aware toolbar actions ----
        // js-line-comment: ---- Sequencer playback ----
        // js-line-comment: ===================== MANNEQUIN VIEWPORT (Animation Editor) =====================
        // js-line-comment: hips (root pivot)
        // js-line-comment: spine -> chest -> head
        // js-line-comment: neck
        // js-line-comment: arms
        // js-line-comment: legs
        // js-line-comment: mirror: when the selected motion state has Mirror on, swap left/right limbs + lateral
        // js-line-comment: sync group: when phase-match is OFF, the right side drifts out of phase (visible foot-slide)
        // js-line-comment: legs
        // js-line-comment: arms (opposite)
        // js-line-comment: --- motion events drive the model: detect playhead crossings, pulse the matching contact ---
        // js-line-comment: body
        // js-line-comment: ---- jump phase clock: crouch -> launch/airborne -> land squash ----
        // js-line-comment: report active sub-state to React (throttled)
        // js-line-comment: root motion: translate forward along facing; camera + ground follow, grid scrolls
        // js-line-comment: additive lean layer (manual Lean param + auto-lean into turns)
        // js-line-comment: IK post-process: foot IK (ankle leveling + ground contact) + look-at
        // js-line-comment: jump overrides: tuck legs + raise arms during air, deep knee bend on crouch/land
        // js-line-comment: bottom time-view playhead + sound-event flash overlay
        // js-line-comment: ===== LEFT RAIL =====
        // js-line-comment: categories (UI folders) — tables view
        // js-line-comment: schemas list — schemas view
        // js-line-comment: ===== CENTER : DATA GRID (table selected) =====
        // js-line-comment: ===== CENTER : SCHEMA EDITOR (schema selected) =====
        // js-line-comment: shared editor context (field inspector reads whichever side is active)
        // js-line-comment: ===== RIGHT : table inspector =====
        // js-line-comment: ===== actions =====
        // js-line-comment: center mode
        // js-line-comment: grid toolbar / table
        // js-line-comment: schema editor center
        // js-line-comment: right tabs
        // js-line-comment: field inspector
        // js-line-comment: table inspector
        // js-line-comment: ---------- DATA MANAGER helpers ----------
        // js-line-comment: Every table gets an auto single-backed manager: keyed, read-only, follows the table.
        // js-line-comment: generated Rust descriptor → array of {key,tokens:[{text,style}]}
        // js-line-comment: key policy
        // js-line-comment: duplicate
        // js-line-comment: filters
        // js-line-comment: projection / output
        // js-line-comment: resolve a source into display chip data
        // js-line-comment: ===== LEFT RAIL: authored managers grouped by owner, auto managers in their own group =====
        // js-line-comment: collapsed by default
        // js-line-comment: ===== CENTER: selected manager composition =====
        // js-line-comment: pipeline stages
        // js-line-comment: ===== INSPECTOR =====
        // js-line-comment: ===== WIZARD =====
        // js-line-comment: review
        // js-line-comment: ===== INSPECTOR: editable controls =====
        // js-line-comment: key field candidates from first table / family source schema
        // js-line-comment: header
        // js-line-comment: inspector
        // js-line-comment: owner (editable dropdown)
        // js-line-comment: output row type (editable text)
        // js-line-comment: key field (editable dropdown)
        // js-line-comment: transforms (toggle chips) + duplicate policy (segmented)
        // js-line-comment: filters (editable / removable)
        // js-line-comment: wizard
        // js-line-comment: ---- transition editing ----
        // js-line-comment: ---- transition condition list (multi-condition with AND/OR) ----
        // js-line-comment: ---- blend space editing ----
        // js-line-comment: ---- motion event editing (Time View) ----
        // js-line-comment: graph scope: root state machine, or the Jump sub-state-machine
        // js-line-comment: effective geometry (with live drag overrides)
        // js-line-comment: --- transitions / connections ---
        // js-line-comment: --- nodes ---
        // js-line-comment: --- pose-pipeline links (state machine output → IK post-process) ---
        // js-line-comment: --- parameters ---
        // js-line-comment: --- motions list ---
        // js-line-comment: --- motion set rows ---
        // js-line-comment: --- skeleton rows ---
        // js-line-comment: --- blend space (editable: drag grid to set point, drag samples to move) ---
        // js-line-comment: shared control builders
        // js-line-comment: --- inspector ---
        // js-line-comment: --- bottom time view (motion events) ---
        // js-line-comment: build dropdown
        // js-line-comment: modal
        // js-line-comment: select dropdown popover
        // js-line-comment: editable live transform (bound to the three.js mesh)
        // js-line-comment: menus
        // js-line-comment: ---- level switcher ----
        // js-line-comment: tools
        // js-line-comment: modes
        // js-line-comment: dock tabs
        // js-line-comment: assets
        // js-line-comment: console
        // js-line-comment: gems
        // js-line-comment: inspector components
        // js-line-comment: ---- status bar helpers ----
        // js-line-comment: ---- viewport overlay pills (functional dropdowns) ----
        // js-line-comment: ---- status bar ----
        // css-comment: this gets exported as style.css and can be used for the default theming
        // css-comment: these are the necessary styles for React/Svelte Flow, they get used by base.css and style.css
        // css-comment: handle styles
        // css-comment: line styles
        // js-block-comment: *
        // js-block-comment: * @license
        // js-block-comment: * Copyright 2010-2021 Three.js Authors
        // js-block-comment: * SPDX-License-Identifier: MIT
        // js-line-comment: Unlike TrackballControls, it maintains the "up" direction object.up (+Y by default).
        // js-line-comment
        // js-line-comment: Orbit - left mouse / touch: one-finger move
        // js-line-comment: Zoom - middle mouse, or mousewheel / touch: two-finger spread or squish
        // js-line-comment: Pan - right mouse, or left mouse + ctrl/meta/shiftKey, or arrow keys / touch: two-finger move
        // js-line-comment: Set to false to disable this control
        // js-line-comment: "target" sets the location of focus, where the object orbits around
        // js-line-comment: How far you can dolly in and out ( PerspectiveCamera only )
        // js-line-comment: How far you can zoom in and out ( OrthographicCamera only )
        // js-line-comment: How far you can orbit vertically, upper and lower limits.
        // js-line-comment: Range is 0 to Math.PI radians.
        // js-line-comment: radians
        // js-line-comment: radians
        // js-line-comment: How far you can orbit horizontally, upper and lower limits.
        // js-line-comment: If set, the interval [ min, max ] must be a sub-interval of [ - 2 PI, 2 PI ], with ( max - min < 2 PI )
        // js-line-comment: radians
        // js-line-comment: radians
        // js-line-comment: Set to true to enable damping (inertia)
        // js-line-comment: If damping is enabled, you must call controls.update() in your animation loop
        // js-line-comment: This option actually enables dollying in and out; left as "zoom" for backwards compatibility.
        // js-line-comment: Set to false to disable zooming
        // js-line-comment: Set to false to disable rotating
        // js-line-comment: Set to false to disable panning
        // js-line-comment: if false, pan orthogonal to world-space direction camera.up
        // js-line-comment: pixels moved per arrow key push
        // js-line-comment: Set to true to automatically rotate around the target
        // js-line-comment: If auto-rotate is enabled, you must call controls.update() in your animation loop
        // js-line-comment: 30 seconds per orbit when fps is 60
        // js-line-comment: The four arrow keys
        // js-line-comment: Mouse buttons
        // js-line-comment: Touch fingers
        // js-line-comment: for reset
        // js-line-comment: the target DOM element for key events
        // js-line-comment
        // js-line-comment: public methods
        // js-line-comment
        // js-line-comment: this method is exposed, but perhaps it would be better if we can make it private...
        // js-line-comment: so camera.up is the orbit axis
        // js-line-comment: rotate offset to "y-axis-is-up" space
        // js-line-comment: angle from z-axis around y-axis
        // js-line-comment: restrict theta to be between desired limits
        // js-line-comment: restrict phi to be between desired limits
        // js-line-comment: restrict radius to be between desired limits
        // js-line-comment: move target to panned location
        // js-line-comment: rotate offset back to "camera-up-vector-is-up" space
        // js-line-comment: update condition is:
        // js-line-comment: min(camera displacement, camera rotation in radians)^2 > EPS
        // js-line-comment: using small-angle approximation cos(x/2) = 1 - x^2 / 8
        // js-line-comment: scope.dispatchEvent( { type: 'dispose' } ); // should this be added here?
        // js-line-comment
        // js-line-comment: internals
        // js-line-comment
        // js-line-comment: current position in spherical coordinates
        // js-line-comment: get X column of objectMatrix
        // js-line-comment: deltaX and deltaY are in pixels; right and down are positive
        // js-line-comment: perspective
        // js-line-comment: half of the fov is center to top of screen
        // js-line-comment: we use only clientHeight here so aspect ratio does not distort speed
        // js-line-comment: orthographic
        // js-line-comment: camera neither orthographic nor perspective
        // js-line-comment
        // js-line-comment: event callbacks - update the object state
        // js-line-comment
        // js-line-comment: yes, height
        // js-line-comment: no-op
        // js-line-comment: prevent the browser from scrolling on cursor keys
        // js-line-comment: yes, height
        // js-line-comment: no-op
        // js-line-comment
        // js-line-comment: event handlers - FSM: listen for events and reset state
        // js-line-comment
        // js-line-comment: TODO touch
        // js-line-comment: TODO touch
        // js-line-comment: TODO touch
        // js-line-comment: Prevent the browser from scrolling.
        // js-line-comment: Manually set the focus since calling preventDefault above
        // js-line-comment: prevents the browser from setting it automatically.
        // js-line-comment: prevent scrolling
        // js-line-comment: prevent scrolling
        // js-line-comment
        // js-line-comment: force an update at start
        // js-line-comment: This set of controls performs orbiting, dollying (zooming), and panning.
        // js-line-comment: Unlike TrackballControls, it maintains the "up" direction object.up (+Y by default).
        // js-line-comment: This is very similar to OrbitControls, another set of touch behavior
        // js-line-comment
        // js-line-comment: Orbit - right mouse, or left mouse + ctrl/meta/shiftKey / touch: two-finger rotate
        // js-line-comment: Zoom - middle mouse, or mousewheel / touch: two-finger spread or squish
        // js-line-comment: Pan - left mouse, or arrow keys / touch: one-finger move
        // js-line-comment: pan orthogonal to world-space direction camera.up
        // js-line-comment: Defined getter, setter and store for a property
        // js-line-comment: Define properties with getters/setter
        // js-line-comment: Setting the defined property will automatically trigger change event
        // js-line-comment: Defined properties are passed down to gizmo and plane
        // js-line-comment: Reusable utility variables
        // js-line-comment: TODO: remove properties unused in plane and gizmo
        // js-line-comment: updateMatrixWorld  updates key transformation variables
        // js-line-comment: Apply translate
        // js-line-comment: Apply translation snap
        // js-line-comment: Apply scale
        // js-line-comment: Apply rotation snap
        // js-line-comment: Apply rotate
        // js-line-comment: Set current object
        // js-line-comment: Detatch from object
        // js-line-comment: TODO: deprecate
        // js-line-comment: mouse / touch event handlers
        // js-line-comment: disable touch scroll
        // js-line-comment
        // js-line-comment: Reusable utility variables
        // js-line-comment: shared materials
        // js-line-comment: Make unique material for each axis/color
        // js-line-comment: reusable geometry
        // js-line-comment: Special geometry for transform helper. If scaled with position vector it spans from [0,0,0] to position
        // js-line-comment: Gizmo definitions - custom hierarchy definitions for setupGizmo() function
        // js-line-comment: Creates an THREE.Object3D with gizmos described in custom hierarchy definition.
        // js-line-comment: name and tag properties are essential for picking and updating logic.
        // js-line-comment: Gizmo creation
        // js-line-comment: Pickers should be hidden always
        // js-line-comment: updateMatrixWorld will update transformations and appearance of individual handles
        // js-line-comment: scale always oriented to local rotation
        // js-line-comment: Show only gizmos for current transform mode
        // js-line-comment: hide aligned to camera
        // js-line-comment: TODO: simplify helpers and consider decoupling from gizmo
        // js-line-comment: If updating helper, skip rest of the loop
        // js-line-comment: Align handles to current local or world rotation
        // js-line-comment: Hide translate and scale axis facing the camera
        // js-line-comment: Flip translate and scale axis ocluded behind another axis
        // js-line-comment: Align handles to current local or world rotation
        // js-line-comment: Hide disabled axes
        // js-line-comment: highlight selected axis
        // js-line-comment
        // js-line-comment: scale always oriented to local rotation
        // js-line-comment: Align the plane for current transform mode, axis and space.
        // js-line-comment: special case for rotate
        // js-line-comment: If in rotate mode, make the plane parallel to camera
        // js-block-comment: esm.sh - codemirror@6.0.1
        // js-block-comment: esm.sh - @codemirror/lang-rust@6.0.1
        // js-block-comment: esm.sh - @codemirror/theme-one-dark@6.1.2
        // js-block-comment: esm.sh - @codemirror/search@6.5.6
        // js-block-comment: esm.sh - @codemirror/commands@6.6.0
        // js-block-comment: esm.sh - react@18.3.1
        // js-block-comment: esm.sh - react-dom@18.3.1/client
        // js-block-comment: esm.sh - @xyflow/react@12.3.6
        // js-block-comment: esm.sh - @codemirror/autocomplete@6.20.3
        // js-block-comment: esm.sh - @codemirror/commands@6.10.4
        // js-block-comment: esm.sh - @codemirror/language@6.12.4
        // js-block-comment: esm.sh - @codemirror/lint@6.9.7
        // js-block-comment: esm.sh - @codemirror/search@6.7.1
        // js-block-comment: esm.sh - @codemirror/state@6.7.0
        // js-block-comment: esm.sh - @codemirror/view@6.43.4
        // js-block-comment: esm.sh - @lezer/rust@1.0.2
        // js-block-comment: esm.sh - @lezer/highlight@1.2.3
        // js-block-comment: esm.sh - crelt@1.0.7
        // js-block-comment: esm.sh - @codemirror/state@6.7.0
        // js-block-comment: esm.sh - @codemirror/view@6.43.4
        // js-block-comment: esm.sh - @lezer/common@1.5.2
        // js-block-comment: esm.sh - react-dom@18.3.1
        // js-block-comment: ! Bundled license information:
        // js-block-comment:
        // js-block-comment: react-dom/cjs/react-dom.production.min.js:
        // js-block-comment: (**
        // js-block-comment: * @license React
        // js-block-comment: * react-dom.production.min.js
        // js-block-comment: *
        // js-block-comment: * Copyright (c) Facebook, Inc. and its affiliates.
        // js-block-comment: *
        // js-block-comment: * This source code is licensed under the MIT license found in the
        // js-block-comment: * LICENSE file in the root directory of this source tree.
        // js-block-comment: *)
        // js-line-comment: # sourceMappingURL=react-dom.mjs.map
        // js-block-comment: esm.sh - @xyflow/system@0.0.47
        // js-line-comment: # sourceMappingURL=system.mjs.map
        // js-block-comment: esm.sh - classcat@5.0.5
        // js-block-comment: esm.sh - react@18.3.1/jsx-runtime
        // js-block-comment: ! Bundled license information:
        // js-block-comment:
        // js-block-comment: react/cjs/react-jsx-runtime.production.min.js:
        // js-block-comment: (**
        // js-block-comment: * @license React
        // js-block-comment: * react-jsx-runtime.production.min.js
        // js-block-comment: *
        // js-block-comment: * Copyright (c) Facebook, Inc. and its affiliates.
        // js-block-comment: *
        // js-block-comment: * This source code is licensed under the MIT license found in the
        // js-block-comment: * LICENSE file in the root directory of this source tree.
        // js-block-comment: *)
        // js-line-comment: # sourceMappingURL=jsx-runtime.mjs.map
        // js-block-comment: esm.sh - react@18.3.1
        // js-block-comment: ! Bundled license information:
        // js-block-comment:
        // js-block-comment: react/cjs/react.production.min.js:
        // js-block-comment: (**
        // js-block-comment: * @license React
        // js-block-comment: * react.production.min.js
        // js-block-comment: *
        // js-block-comment: * Copyright (c) Facebook, Inc. and its affiliates.
        // js-block-comment: *
        // js-block-comment: * This source code is licensed under the MIT license found in the
        // js-block-comment: * LICENSE file in the root directory of this source tree.
        // js-block-comment: *)
        // js-line-comment: # sourceMappingURL=react.mjs.map
        // js-block-comment: esm.sh - zustand@4.5.7/shallow
        // js-block-comment: esm.sh - zustand@4.5.7/traditional
        // js-block-comment: esm.sh - @codemirror/view@6.43.4
        // js-block-comment: esm.sh - @codemirror/state@6.7.0
        // js-block-comment: esm.sh - @codemirror/view@6.43.4
        // js-block-comment: esm.sh - @lezer/common@1.5.2
        // js-block-comment: esm.sh - style-mod@4.1.3
        // js-block-comment: esm.sh - @codemirror/view@6.43.4
        // js-block-comment: esm.sh - @codemirror/view@6.43.4
        // js-block-comment: esm.sh - @marijn/find-cluster-break@1.0.3
        // js-block-comment: esm.sh - crelt@1.0.7
        // js-block-comment: esm.sh - style-mod@4.1.3
        // js-block-comment: esm.sh - w3c-keyname@2.2.8
        // js-block-comment: esm.sh - @lezer/lr@1.4.10
        // js-block-comment: esm.sh - @lezer/common@1.5.2
        // js-block-comment: esm.sh - scheduler@0.23.2
        // js-block-comment: esm.sh - d3-drag@3.0.0
        // js-block-comment: esm.sh - d3-selection@3.0.0
        // js-block-comment: esm.sh - d3-zoom@3.0.0
        // js-block-comment: esm.sh - use-sync-external-store@1.6.0/shim/with-selector
        // js-block-comment: esm.sh - zustand@4.5.7/vanilla
        // js-line-comment: # sourceMappingURL=vanilla.mjs.map
        // js-block-comment: esm.sh - @lezer/common@1.5.2
        // js-block-comment: esm.sh - d3-dispatch@3.0.1
        // js-block-comment: esm.sh - d3-selection@3.0.0
        // js-block-comment: esm.sh - d3-interpolate@3.0.1
        // js-block-comment: esm.sh - d3-transition@3.0.1
        // js-block-comment: esm.sh - use-sync-external-store@1.6.0/shim
        // js-block-comment: ! Bundled license information:
        // js-block-comment:
        // js-block-comment: use-sync-external-store/cjs/use-sync-external-store-shim.production.js:
        // js-block-comment: (**
        // js-block-comment: * @license React
        // js-block-comment: * use-sync-external-store-shim.production.js
        // js-block-comment: *
        // js-block-comment: * Copyright (c) Meta Platforms, Inc. and affiliates.
        // js-block-comment: *
        // js-block-comment: * This source code is licensed under the MIT license found in the
        // js-block-comment: * LICENSE file in the root directory of this source tree.
        // js-block-comment: *)
        // js-line-comment: # sourceMappingURL=shim.mjs.map
        // js-block-comment: esm.sh - d3-color@3.1.0
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/active
        // js-line-comment: # sourceMappingURL=active.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/interrupt
        // js-line-comment: # sourceMappingURL=interrupt.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/selection/index
        // js-line-comment: # sourceMappingURL=index.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/index
        // js-line-comment: # sourceMappingURL=index.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/schedule
        // js-line-comment: # sourceMappingURL=schedule.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/selection/interrupt
        // js-line-comment: # sourceMappingURL=interrupt.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/selection/transition
        // js-line-comment: # sourceMappingURL=transition.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/attr
        // js-line-comment: # sourceMappingURL=attr.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/attrTween
        // js-line-comment: # sourceMappingURL=attrTween.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/delay
        // js-line-comment: # sourceMappingURL=delay.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/duration
        // js-line-comment: # sourceMappingURL=duration.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/ease
        // js-line-comment: # sourceMappingURL=ease.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/easeVarying
        // js-line-comment: # sourceMappingURL=easeVarying.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/filter
        // js-line-comment: # sourceMappingURL=filter.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/merge
        // js-line-comment: # sourceMappingURL=merge.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/on
        // js-line-comment: # sourceMappingURL=on.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/remove
        // js-line-comment: # sourceMappingURL=remove.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/select
        // js-line-comment: # sourceMappingURL=select.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/selectAll
        // js-line-comment: # sourceMappingURL=selectAll.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/selection
        // js-line-comment: # sourceMappingURL=selection.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/style
        // js-line-comment: # sourceMappingURL=style.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/styleTween
        // js-line-comment: # sourceMappingURL=styleTween.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/text
        // js-line-comment: # sourceMappingURL=text.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/textTween
        // js-line-comment: # sourceMappingURL=textTween.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/transition
        // js-line-comment: # sourceMappingURL=transition.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/tween
        // js-line-comment: # sourceMappingURL=tween.mjs.map
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/end
        // js-line-comment: # sourceMappingURL=end.mjs.map
        // js-block-comment: esm.sh - d3-timer@3.0.1
        // js-block-comment: esm.sh - d3-ease@3.0.1
        // js-block-comment: esm.sh - d3-transition@3.0.1/src/transition/interpolate
        // js-line-comment: # sourceMappingURL=interpolate.mjs.map
        // source-metadata: data-dc-script data-props={"$preview":{"width":300,"height":420},"config":{"editor":"enum","options":["Debug","Development","Profile","Shipping"],"default":"Development","tsType":"string","section":"State"},"target":{"editor":"enum","options":["Windows","Linux","macOS","Android","Web"],"default":"Windows","tsType":"string","section":"State"},"open":{"editor":"boolean","default":true,"tsType":"boolean","section":"State"},"configShort":{"editor":"text","default":"Dev","tsType":"string","section":"Content"},"onToggle":{"editor":null,"tsType":"() => void"},"onClose":{"editor":null,"tsType":"() => void"},"onConfig":{"editor":null,"tsType":"(name: string) => void"},"onTarget":{"editor":null,"tsType":"(name: string) => void"},"onAction":{"editor":null,"tsType":"(label: string) => void"}}
        // source-metadata: data-dc-script data-props={"$preview":{"width":46,"height":48},"icon":{"editor":"text","default":"view_in_ar","tsType":"string","section":"Content"},"title":{"editor":"text","default":"Scene","tsType":"string","section":"Content"},"active":{"editor":"boolean","default":true,"tsType":"boolean","section":"State"},"onClick":{"editor":null,"tsType":"() => void"}}
        // source-metadata: data-dc-script data-props={"$preview":{"width":180,"height":40},"playState":{"editor":"enum","options":["stopped","paused","playing"],"default":"stopped","tsType":"'stopped'|'paused'|'playing'","section":"State"},"simulating":{"editor":"boolean","default":false,"tsType":"boolean","section":"State"},"onPlay":{"editor":null,"tsType":"() => void"},"onStop":{"editor":null,"tsType":"() => void"},"onStep":{"editor":null,"tsType":"() => void"},"onSimulate":{"editor":null,"tsType":"() => void"}}
        // source logic: dialect=dc, type=text/x-dc, body omitted
        // source logic: dialect=dc, type=text/x-dc, data-props={"$preview":{"width":300,"height":420},"config":{"editor":"enum","options":["Debug","Development","Profile","Shipping"],"default":"Development","tsType":"string","section":"State"},"target":{"editor":"enum","options":["Windows","Linux","macOS","Android","Web"],"default":"Windows","tsType":"string","section":"State"},"open":{"editor":"boolean","default":true,"tsType":"boolean","section":"State"},"configShort":{"editor":"text","default":"Dev","tsType":"string","section":"Content"},"onToggle":{"editor":null,"tsType":"() => void"},"onClose":{"editor":null,"tsType":"() => void"},"onConfig":{"editor":null,"tsType":"(name: string) => void"},"onTarget":{"editor":null,"tsType":"(name: string) => void"},"onAction":{"editor":null,"tsType":"(label: string) => void"}}, body omitted
        // source logic: dialect=dc, type=text/x-dc, data-props={"$preview":{"width":46,"height":48},"icon":{"editor":"text","default":"view_in_ar","tsType":"string","section":"Content"},"title":{"editor":"text","default":"Scene","tsType":"string","section":"Content"},"active":{"editor":"boolean","default":true,"tsType":"boolean","section":"State"},"onClick":{"editor":null,"tsType":"() => void"}}, body omitted
        // source logic: dialect=dc, type=text/x-dc, data-props={"$preview":{"width":180,"height":40},"playState":{"editor":"enum","options":["stopped","paused","playing"],"default":"stopped","tsType":"'stopped'|'paused'|'playing'","section":"State"},"simulating":{"editor":"boolean","default":false,"tsType":"boolean","section":"State"},"onPlay":{"editor":null,"tsType":"() => void"},"onStop":{"editor":null,"tsType":"() => void"},"onStep":{"editor":null,"tsType":"() => void"},"onSimulate":{"editor":null,"tsType":"() => void"}}, body omitted
        let theme = _cx.theme().clone();
        gpui::div()
            .m(gpui::px(0.0))
            .p(gpui::px(0.0))
            .size_full()
            .h_full()
            .bg(theme.background)
            .on_action(_cx.listener(Self::toggle_left_dock))
            .on_action(_cx.listener(Self::toggle_asset_browser_dock))
            .on_action(_cx.listener(Self::toggle_right_dock))
            .on_action(_cx.listener(Self::toggle_bottom_console_dock))
            .on_action(_cx.listener(Self::toggle_bottom_session_dock))
            .on_action(_cx.listener(Self::show_graph_workspace))
            .on_action(_cx.listener(Self::route_material_source_selection))
            .on_action(_cx.listener(Self::route_script_source_selection))
            .on_action(_cx.listener(Self::open_preferences_action))
            .on_action(_cx.listener(Self::show_about_dialog))
            .on_action(_cx.listener(Self::dismiss_overlay_action))
            .on_action(_cx.listener(Self::reset_layout_action))
            .child({
                // source tag: x-dc
                gpui::div()
                    .size_full()
                    .m(gpui::px(0.0))
                    .p(gpui::px(0.0))
                    .child({
                        // source tag: div
                        // source region: editor
                        let editor_column = gpui::div()
                            .m(gpui::px(0.0))
                            .p(gpui::px(0.0))
                            .flex()
                            .flex_col()
                            .h_full()
                            .w_full()
                            .overflow_hidden()
                            .bg(theme.background)
                            .text_color(theme.foreground)
                            .font_family(theme.font_family.clone())
                            .text_size(gpui::px(12.0))
                            .line_height(gpui::relative(1.35))
                            .relative();
                        // htmlswap source-order: 0
                        // htmlswap paint-order: 0 z-index: 60
                        let titlebar = self.titlebar_region.clone();
                        // htmlswap source-order: 1
                        // htmlswap paint-order: 1 z-index: 40
                        let toolbar = self.toolbar_region.clone();
                        // htmlswap source-order: 2
                        // htmlswap paint-order: 2 z-index: auto
                        let workspace = gpui::div()
                            .flex_auto()
                            .flex()
                            .min_w(gpui::px(0.0))
                            .min_h(gpui::px(0.0))
                            .child(self.activity_region.clone())
                            .child(self.workspace_region.clone());
                        // htmlswap source-order: 3
                        // htmlswap paint-order: 3 z-index: 30
                        let status_bar = self.status_region.clone();
                        // htmlswap control-flow source-order: 4
                        // htmlswap paint-order: 4 z-index: 200
                        // source control-flow: if, expr=modalOpen, placeholder=hint-placeholder-val="{{ false }}"
                        let editor_column = editor_column
                            .child(titlebar)
                            .child(toolbar)
                            .child(workspace)
                            .child(status_bar)
                            .child(self.modal_region.clone())
                            // htmlswap control-flow source-order: 5
                            // htmlswap paint-order: 5 z-index: 210
                            .child(self.render_top_menu_overlay(_cx));
                        crate::perf::record_elapsed(
                            crate::perf::FRAME_GPUI_RENDER,
                            perf_render_started,
                        );
                        editor_column
                    })
            })
    }
}
