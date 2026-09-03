//! Scripting mode: left file tree (script canvases + text script assets),
//! center script/graph editor, right outline.
//!
//! All three panels render [`EditorScriptingProjection`] published by
//! editor-core (plus [`EditorAssetBrowserStatus`], read directly for the
//! always-fresh text-asset tree — see `scripting_projection` module docs) and
//! dispatch typed `Script*` / graph actions; no graph controller or
//! project-host types appear here. Layout follows the Aether editor design
//! (`design/aether-handoff/project/Aether Editor.dc.html`, Script Editor
//! region): file tree left, code/graph canvas center, outline right.
//!
//! Wave D2 scope: script canvases open through the shared visual graph
//! canvas ([`VisualGraphPanel`], same pattern as `MaterialWorkbenchPanel`).
//! Text script sources have no content-load RPC yet (`EditorAssetSourcePreview`
//! only classifies a preview kind, it never loads bytes), so the center
//! workbench renders an honest file card with a real
//! "Open in External Editor" action instead of a fake in-app code viewer.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
};
use gpui_component::dock::Panel;
use gpui_component::theme::Theme;
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, h_flex, v_flex};

use crate::panels::asset_browser::EditorAssetBrowserStatus;
use crate::panels::graph::VisualGraphPanel;
use crate::panels::kit;
use crate::panels::modes::mode_panel;
use crate::panels::scripting_projection::{
    EditorScriptingProjection, ScriptFolderProjection, ScriptOutlineRowKind,
    ScriptTextFileRowProjection, ScriptWorkbenchProjection, script_text_folders,
};

fn scripting(cx: &App) -> EditorScriptingProjection {
    cx.try_global::<EditorScriptingProjection>()
        .cloned()
        .unwrap_or_default()
}

fn element_id(prefix: &str, key: &str) -> SharedString {
    SharedString::from(format!(
        "{prefix}-{}",
        key.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    ))
}

// ---------------------------------------------------------------------------
// Left rail: file tree (script canvases + text script assets)
// ---------------------------------------------------------------------------

fn render_script_files(cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let data = scripting(cx);
    let text_folders = cx
        .try_global::<EditorAssetBrowserStatus>()
        .map(|status| script_text_folders(status, data.selected_key.as_deref()))
        .unwrap_or_default();

    let has_any = !data.graph_scripts.is_empty() || !text_folders.is_empty();

    let body: gpui::AnyElement = if has_any {
        v_flex()
            .gap_2()
            .when(!data.graph_scripts.is_empty(), |this| {
                this.child(
                    v_flex()
                        .gap_0p5()
                        .child(kit::section_label("Graph Scripts", &theme))
                        .children(data.graph_scripts.iter().map(|row| {
                            let key = row.key.clone();
                            kit::list_row(
                                element_id("script-graph", &row.key),
                                &theme,
                                row.selected,
                            )
                            .child(
                                Icon::new(IconName::Network)
                                    .with_size(gpui::px(14.0))
                                    .text_color(if row.has_diagnostics {
                                        theme.danger
                                    } else if row.unsaved_changes {
                                        theme.warning
                                    } else {
                                        theme.muted_foreground
                                    }),
                            )
                            .child(kit::row_name(row.name.clone(), &theme))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                window.dispatch_action(
                                    Box::new(crate::actions::SelectScriptFile { key: key.clone() }),
                                    cx,
                                );
                            })
                        })),
                )
            })
            .children(
                text_folders
                    .iter()
                    .map(|folder| render_script_text_folder(folder, &theme)),
            )
            .into_any_element()
    } else {
        let sub = if cx.try_global::<EditorAssetBrowserStatus>().is_some() {
            "no script canvases open and no script source root is registered for this project \
             yet (azoth.toml scripts root is not wired into the watched asset roots)"
        } else {
            "waiting for the asset-processor workspace view"
        };
        kit::empty_state("No scripts found.", Some(sub.to_owned()), &theme).into_any_element()
    };

    kit::panel_shell(
        "Scripts",
        Some("script canvases · script assets".to_owned()),
        body,
        &theme,
    )
}

fn render_script_text_folder(folder: &ScriptFolderProjection, theme: &Theme) -> impl IntoElement {
    v_flex()
        .gap_0p5()
        .child(kit::section_label(folder.name.clone(), theme))
        .children(
            folder
                .files
                .iter()
                .map(|file| render_script_text_row(file, theme)),
        )
}

fn render_script_text_row(file: &ScriptTextFileRowProjection, theme: &Theme) -> impl IntoElement {
    let key = file.key.clone();
    kit::list_row(element_id("script-text", &file.key), theme, file.selected)
        .child(kit::row_icon(
            IconName::File,
            if file.has_diagnostics {
                theme.danger
            } else {
                theme.muted_foreground
            },
        ))
        .child(kit::row_name(file.name.clone(), theme))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(
                Box::new(crate::actions::SelectScriptFile { key: key.clone() }),
                cx,
            );
        })
}

mode_panel!(
    ScriptFilesPanel,
    "script-files",
    "Files",
    render_script_files,
    Some("folder_open"),
    kit::TabTone::Muted
);

// ---------------------------------------------------------------------------
// Right rail: outline
// ---------------------------------------------------------------------------

fn render_script_outline(cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let data = scripting(cx);

    let body: gpui::AnyElement = if data.outline.is_empty() {
        kit::hint_text(data.outline_empty_reason, &theme).into_any_element()
    } else {
        v_flex()
            .gap_0p5()
            .children(data.outline.iter().map(|row| {
                let (icon, color) = match row.kind {
                    ScriptOutlineRowKind::Node => (IconName::Network, theme.info),
                    ScriptOutlineRowKind::Port { input: true } => {
                        (IconName::ArrowRight, theme.success)
                    }
                    ScriptOutlineRowKind::Port { input: false } => {
                        (IconName::ArrowRight, theme.warning)
                    }
                    ScriptOutlineRowKind::Diagnostic => (IconName::TriangleAlert, theme.danger),
                };
                h_flex()
                    .h(gpui::px(kit::ROW_H))
                    .items_center()
                    .gap_1p5()
                    .pl(kit::indent(row.depth, 14.0, 8.0))
                    .pr_2()
                    .child(kit::row_icon(icon, color))
                    .child(kit::row_name(row.label.clone(), &theme))
            }))
            .into_any_element()
    };

    kit::panel_shell("Outline", None, body, &theme)
}

mode_panel!(
    ScriptOutlinePanel,
    "script-outline",
    "Outline",
    render_script_outline,
    Some("list"),
    kit::TabTone::Muted
);

// ---------------------------------------------------------------------------
// Center: script/graph workbench
// ---------------------------------------------------------------------------

/// Center workbench: script canvas tabs + Save over the shared visual graph
/// canvas.
///
/// Script canvases route the same way `.azmat.ron` graphs do for Materials.
/// Text script sources instead get an honest file card with a real "Open in
/// External Editor" action — no content-load RPC exists to back an in-app
/// viewer yet (see module docs).
pub struct ScriptEditorPanel {
    focus_handle: FocusHandle,
    graph_canvas: Entity<VisualGraphPanel>,
}

impl ScriptEditorPanel {
    pub const NAME: &'static str = "script-editor";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let graph_canvas = cx.new(|cx| VisualGraphPanel::init(window, cx));
        Self {
            focus_handle: cx.focus_handle(),
            graph_canvas,
        }
    }
}

impl Render for ScriptEditorPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) =
            crate::panels::render_project_host_connection_placeholder("Script", cx)
        {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let data = scripting(cx);
        let hosts_graph = matches!(data.workbench, ScriptWorkbenchProjection::Graph { .. });

        let toolbar = kit::panel_toolbar(&theme)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .when(data.tabs.is_empty(), |this| {
                        this.child(kit::panel_subtitle("no open script canvases", &theme))
                    })
                    .children(data.tabs.iter().map(|tab| {
                        let label = if tab.unsaved_changes {
                            format!("{}*", tab.label)
                        } else {
                            tab.label.clone()
                        };
                        let key = tab.key.clone();
                        kit::filter_chip(
                            element_id("script-tab", &tab.key),
                            label,
                            None,
                            None,
                            None,
                            tab.selected,
                            &theme,
                        )
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            window.dispatch_action(
                                Box::new(crate::actions::SelectScriptFile { key: key.clone() }),
                                cx,
                            );
                        })
                    })),
            )
            .when(hosts_graph, |this| {
                this.child(kit::toolbar_icon_button(
                    "script-save-graph",
                    IconName::Save,
                    "Save script canvas",
                    crate::actions::SaveGraphDocument,
                    &theme,
                ))
            });

        let breadcrumb = (!data.breadcrumb.is_empty())
            .then(|| kit::panel_subtitle(data.breadcrumb.join(" / "), &theme));

        let body: gpui::AnyElement = match &data.workbench {
            ScriptWorkbenchProjection::Graph { .. } => div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .child(self.graph_canvas.clone())
                .into_any_element(),
            ScriptWorkbenchProjection::Text(row) => {
                render_script_text_card(row, &theme).into_any_element()
            }
            ScriptWorkbenchProjection::Empty { message } => {
                kit::empty_state(message.clone(), None, &theme).into_any_element()
            }
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(toolbar)
            .children(breadcrumb)
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .when(!hosts_graph, |this| this.p_4().gap_2p5())
                    .child(body),
            )
            .into_any_element()
    }
}

fn render_script_text_card(row: &ScriptTextFileRowProjection, theme: &Theme) -> impl IntoElement {
    let source_root = row.source_root.clone();
    let source_path = row.source_path.clone();
    v_flex()
        .gap_2p5()
        .child(
            h_flex()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::File).text_color(theme.info))
                .child(
                    div()
                        .text_size(gpui::px(13.0))
                        .text_color(theme.foreground)
                        .child(crate::naming::display_name(&row.source_path).into_owned()),
                ),
        )
        .child(kit::status_row("Kind", row.schema_label.clone(), theme))
        .child(kit::status_row("Status", row.status_label.clone(), theme))
        .child(kit::hint_text(
            "No in-editor content preview yet — a script source-read RPC does not exist. Edit \
             this file in your external editor.",
            theme,
        ))
        .child(
            div()
                .id("script-open-external")
                .flex_none()
                .px_3()
                .py_1p5()
                .rounded(gpui::px(6.0))
                .bg(theme.accent)
                .text_color(theme.accent_foreground)
                .text_size(gpui::px(12.0))
                .cursor_pointer()
                .child("Open in External Editor")
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(crate::actions::OpenScriptInExternalEditor {
                            source_root: source_root.clone(),
                            source_path: source_path.clone(),
                        }),
                        cx,
                    );
                }),
        )
}

impl Focusable for ScriptEditorPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for ScriptEditorPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("code"), "Script", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for ScriptEditorPanel {}
