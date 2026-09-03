//! Scene Layers panel
//!
//! O3DE-style scene layers adapted to `AZoth`'s authored model: each authored
//! document is a layer — a named, file-backed container of objects (the same
//! "a layer is its own file for team collaboration" model O3DE uses). Each layer
//! carries an object count and editor-side visibility/lock that inherit to the
//! layer's objects: hiding a layer hides its objects in the in-process viewport.
//! O3DE persists no per-layer color, so the swatch here is derived deterministically
//! from the document id for visual distinction only.
//!
//! This panel renders the real authored outline; the editor shell owns the
//! visibility/lock state ([`EditorLayerVisibility`]) and applies it to the scene.

use std::collections::BTreeSet;

use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{ActiveTheme, Icon, IconName, StyledExt, h_flex, v_flex};

use crate::panels::{EditorAuthoredOutline, is_scene_document, kit};

/// Editor-owned per-layer (per authored document) visibility and lock state.
///
/// The editor shell mutates this in response to [`crate::actions::SetLayerVisibility`]
/// / [`crate::actions::SetLayerLock`] and applies it to the in-process viewport
/// scene; this panel only reads it.
#[derive(Debug, Clone, Default)]
pub struct EditorLayerVisibility {
    /// Document ids whose layer (and objects) are hidden.
    pub hidden: BTreeSet<String>,
    /// Document ids whose layer (and objects) are locked.
    pub locked: BTreeSet<String>,
}

impl gpui::Global for EditorLayerVisibility {}

/// Stable visibility key for one authored object inside a document.
#[must_use]
pub fn authored_object_visibility_key(document_id: &str, object_id: &str) -> String {
    format!("{document_id}/{object_id}")
}

/// Deterministic swatch palette (O3DE stores no per-layer color, so this is a
/// stable visual aid derived from the document id).
const LAYER_SWATCHES: &[u32] = &[
    0x7a_8aa0, 0x8f_c97e, 0xd6_a23b, 0xd6_c14a, 0xb7_8fd6, 0x5a_c0c0, 0xd9_645e, 0x41_88e0,
];

/// Stable color for a layer derived from its document id.
#[must_use]
pub fn layer_color(document_id: &str) -> u32 {
    let hash = document_id.bytes().fold(0u32, |acc, byte| {
        acc.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    LAYER_SWATCHES[(hash as usize) % LAYER_SWATCHES.len()]
}

/// Human-readable layer name: the document's file stem, falling back to the id.
#[must_use]
pub fn layer_name(source_path: &str, document_id: &str) -> String {
    let trimmed = source_path.trim();
    if trimmed.is_empty() {
        return document_id.to_string();
    }
    let file = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed);
    let stem = file.rsplit_once('.').map_or(file, |(stem, _ext)| stem);
    if stem.is_empty() {
        document_id.to_string()
    } else {
        stem.to_string()
    }
}

/// One layer row resolved from the authored outline + editor layer state.
pub struct AuthoredLayerRow {
    pub document_id: String,
    pub name: String,
    pub color: u32,
    pub count: u32,
    pub unsaved: bool,
    pub hidden: bool,
    pub locked: bool,
}

#[must_use]
pub fn authored_layer_rows(
    outline: &EditorAuthoredOutline,
    state: Option<&EditorLayerVisibility>,
) -> Vec<AuthoredLayerRow> {
    outline
        .data
        .documents
        .iter()
        .filter(|document| is_scene_document(document))
        .map(|document| AuthoredLayerRow {
            name: layer_name(&document.source_path, &document.document_id),
            color: layer_color(&document.document_id),
            count: document.object_count,
            unsaved: document.unsaved_changes,
            hidden: state.is_some_and(|s| s.hidden.contains(&document.document_id)),
            locked: state.is_some_and(|s| s.locked.contains(&document.document_id)),
            document_id: document.document_id.clone(),
        })
        .collect()
}

/// Scene Layers panel.
pub struct SceneLayersPanel {
    focus_handle: FocusHandle,
    /// The active layer (target for new objects); a document id.
    active_document_id: Option<String>,
}

impl SceneLayersPanel {
    pub const NAME: &'static str = "scene-layers";

    pub fn init(_window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            active_document_id: None,
        }
    }

    /// Resolve the layer rows from the current authored outline + layer state.
    fn rows(cx: &App) -> Vec<AuthoredLayerRow> {
        let Some(outline) = cx.try_global::<EditorAuthoredOutline>() else {
            return Vec::new();
        };
        authored_layer_rows(outline, cx.try_global::<EditorLayerVisibility>())
    }
}

impl Render for SceneLayersPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows = Self::rows(cx);
        let active_id = self.active_document_id.clone();

        let header = render_layers_header(rows.len(), &theme);

        if rows.is_empty() {
            return v_flex()
                .size_full()
                .bg(theme.sidebar)
                .child(header)
                .child(render_no_layers_state(&theme));
        }

        let active_name = active_id
            .as_ref()
            .and_then(|id| rows.iter().find(|row| &row.document_id == id))
            .or_else(|| rows.first())
            .map(|row| row.name.clone())
            .unwrap_or_default();

        let mut list = v_flex()
            .id("layers-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .py(px(4.0));

        for row in rows {
            let active = active_id.as_deref() == Some(row.document_id.as_str());
            list = list.child(render_layer_row(&row, active, &theme, cx));
        }

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(header)
            .child(list)
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(6.0))
                    .h(px(24.0))
                    .px(px(10.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .font_family(theme.mono_font_family.clone())
                    .text_size(gpui::Rems(0.6))
                    .text_color(theme.muted_foreground)
                    .child(
                        Icon::new(IconName::Eye)
                            .size(px(13.0))
                            .text_color(theme.accent),
                    )
                    .child(format!("Active - {active_name}")),
            )
    }
}

/// Panel header: the title, the layer count, and the spacer that pushes any
/// future header controls to the right edge.
fn render_layers_header(
    layer_count: usize,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(10.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(gpui::Rems(0.6))
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child("SCENE LAYERS"),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.6))
                .text_color(theme.muted_foreground)
                .child(layer_count.to_string()),
        )
        .child(div().flex_1())
}

/// Body shown when the authored outline carries no scene documents.
fn render_no_layers_state(theme: &gpui_component::theme::Theme) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(px(6.0))
        .child(
            Icon::new(IconName::LayoutDashboard)
                .size(px(24.0))
                .text_color(theme.border),
        )
        .child(
            div()
                .text_size(gpui::Rems(0.72))
                .text_color(theme.muted_foreground)
                .child("No layers"),
        )
        .child(
            div()
                .text_size(gpui::Rems(0.62))
                .text_color(theme.muted_foreground)
                .child("Authored documents appear here as layers."),
        )
}

/// One layer row: swatch, name, unsaved dot, ACTIVE badge, object count, and
/// the lock / visibility toggles that dispatch back to the editor shell.
fn render_layer_row(
    row: &AuthoredLayerRow,
    active: bool,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, SceneLayersPanel>,
) -> impl IntoElement {
    let name_color = if active {
        theme.foreground
    } else if row.hidden {
        theme.muted_foreground
    } else {
        theme.sidebar_foreground
    };
    let id_for_active = row.document_id.clone();

    h_flex()
        .id(gpui::SharedString::from(format!(
            "layer-{}",
            row.document_id
        )))
        .items_center()
        .gap(px(8.0))
        .h(px(28.0))
        .px(px(10.0))
        .when(active, |this| this.bg(theme.list_active))
        .when(!active, |this| this.hover(|s| s.bg(theme.list_hover)))
        .child(
            div()
                .flex_none()
                .size(px(9.0))
                .rounded_full()
                .bg(gpui::rgb(row.color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(gpui::Rems(0.74))
                .text_color(name_color)
                .child(row.name.clone()),
        )
        .when(row.unsaved, |this| {
            this.child(
                div()
                    .flex_none()
                    .size(px(5.0))
                    .rounded_full()
                    .bg(theme.warning),
            )
        })
        .when(active, |this| {
            this.child(
                div()
                    .flex_none()
                    .px(px(5.0))
                    .rounded(px(3.0))
                    .bg(theme.list_active_border.opacity(0.16))
                    .text_size(gpui::Rems(0.54))
                    .font_semibold()
                    .text_color(theme.accent)
                    .child("ACTIVE"),
            )
        })
        .child(
            div()
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.6))
                .text_color(theme.muted_foreground)
                .child(row.count.to_string()),
        )
        .child(render_layer_lock_toggle(row, theme, cx))
        .child(render_layer_visibility_toggle(row, theme, cx))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.active_document_id = Some(id_for_active.clone());
            cx.notify();
        }))
}

/// Lock toggle for one layer; dispatches [`crate::actions::SetLayerLock`] with
/// the state the click is asking for.
fn render_layer_lock_toggle(
    row: &AuthoredLayerRow,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, SceneLayersPanel>,
) -> impl IntoElement {
    let document_id = row.document_id.clone();
    let next_locked = !row.locked;
    div()
        .id(gpui::SharedString::from(format!(
            "layer-lock-{}",
            row.document_id
        )))
        .flex_none()
        .child(
            Icon::new(if row.locked {
                IconName::PanelLeftClose
            } else {
                IconName::PanelLeftOpen
            })
            .size(px(14.0))
            .text_color(if row.locked {
                theme.warning
            } else {
                theme.muted_foreground
            }),
        )
        .on_click(cx.listener(move |_, _, window, cx| {
            window.dispatch_action(
                Box::new(crate::actions::SetLayerLock {
                    document_id: document_id.clone(),
                    locked: next_locked,
                }),
                cx,
            );
            cx.stop_propagation();
        }))
}

/// Visibility toggle for one layer; dispatches
/// [`crate::actions::SetLayerVisibility`] with the state the click is asking
/// for (a hidden layer's eye makes it visible).
fn render_layer_visibility_toggle(
    row: &AuthoredLayerRow,
    theme: &gpui_component::theme::Theme,
    cx: &Context<'_, SceneLayersPanel>,
) -> impl IntoElement {
    let document_id = row.document_id.clone();
    let next_visible = row.hidden;
    div()
        .id(gpui::SharedString::from(format!(
            "layer-vis-{}",
            row.document_id
        )))
        .flex_none()
        .child(
            Icon::new(if row.hidden {
                IconName::EyeOff
            } else {
                IconName::Eye
            })
            .size(px(14.0))
            .text_color(theme.muted_foreground),
        )
        .on_click(cx.listener(move |_, _, window, cx| {
            window.dispatch_action(
                Box::new(crate::actions::SetLayerVisibility {
                    document_id: document_id.clone(),
                    visible: next_visible,
                }),
                cx,
            );
            cx.stop_propagation();
        }))
}

impl Focusable for SceneLayersPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for SceneLayersPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        kit::tab_title(Some("layers"), "Layers", kit::TabTone::Default)
    }
}

impl gpui::EventEmitter<PanelEvent> for SceneLayersPanel {}
