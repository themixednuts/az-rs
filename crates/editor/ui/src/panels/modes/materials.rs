//! Materials mode: left node palette, center material graph canvas, right
//! preview + parameters.
//!
//! All three panels render [`EditorMaterialsProjection`] published by
//! editor-core and dispatch typed `Material*` / graph actions; no graph
//! controller or project-host types appear here. Layout follows the Aether
//! editor design (`design/aether-handoff/project/Aether Editor.dc.html`,
//! Material Editor region) and the rendering brief B3 v1 preview contract:
//! the preview swatch is a styled div parameterized from authored property
//! values (base color → tint, roughness → highlight size, metallic →
//! contrast). A real mesh preview (B3 v2) is a documented follow-up on the
//! in-process renderer.

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, Rgba, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Window, div, px, relative,
};
use gpui_component::dock::Panel;
use gpui_component::theme::Theme;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, h_flex,
    input::{Input, InputEvent, InputState},
    scroll::ScrollableElement,
    v_flex,
};

use crate::panels::graph::VisualGraphPanel;
use crate::panels::kit;
use crate::panels::materials_projection::{
    EditorMaterialsProjection, MaterialPaletteProjection, MaterialParamControlProjection,
    MaterialParamRowProjection, MaterialPreviewMaterialProjection, MaterialPreviewShape,
};
use crate::panels::modes::mode_panel;

fn materials(cx: &App) -> EditorMaterialsProjection {
    cx.try_global::<EditorMaterialsProjection>()
        .cloned()
        .unwrap_or_default()
}

fn element_id(prefix: &str, key: &str) -> SharedString {
    SharedString::from(format!(
        "{prefix}-{}",
        key.replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    ))
}

fn hsla_from_rgba(rgba: [f32; 4]) -> Hsla {
    Rgba {
        r: rgba[0],
        g: rgba[1],
        b: rgba[2],
        a: rgba[3],
    }
    .into()
}

const fn with_lightness(color: Hsla, lightness: f32) -> Hsla {
    Hsla {
        l: lightness.clamp(0.0, 1.0),
        ..color
    }
}

/// Palette category tint. Semantic mapping onto theme tokens (mirrors the
/// design's per-category node colors) — no hardcoded chrome colors.
fn palette_category_tint(name: &str, theme: &Theme) -> Hsla {
    match name {
        "Input" => theme.info,
        "Math" => theme.success,
        "Output" => theme.warning,
        "Constants" => theme.accent,
        "Texture" => theme.cyan,
        _ => theme.muted_foreground,
    }
}

// ---------------------------------------------------------------------------
// Left rail: node palette
// ---------------------------------------------------------------------------

pub struct MaterialPalettePanel {
    focus_handle: FocusHandle,
    filter_input: Entity<InputState>,
    /// Last filter pushed into the input; resyncs when the projection filter
    /// changes from outside this panel.
    last_projected_filter: String,
    _subscriptions: Vec<Subscription>,
}

impl MaterialPalettePanel {
    pub const NAME: &'static str = "material-palette";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let filter_input = cx.new(|cx| InputState::new(window, cx).placeholder("Add node…"));
        let subscriptions = vec![cx.subscribe_in(
            &filter_input,
            window,
            |_this: &mut Self, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let filter = input.read(cx).value().to_string();
                    window.dispatch_action(
                        Box::new(crate::actions::SetMaterialPaletteFilter { filter }),
                        cx,
                    );
                }
            },
        )];
        Self {
            focus_handle: cx.focus_handle(),
            filter_input,
            last_projected_filter: String::new(),
            _subscriptions: subscriptions,
        }
    }
}

impl Render for MaterialPalettePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) =
            crate::panels::render_project_host_connection_placeholder("Palette", cx)
        {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let data = materials(cx);

        if self.last_projected_filter != data.palette.filter {
            self.last_projected_filter.clone_from(&data.palette.filter);
            let value = data.palette.filter.clone();
            self.filter_input.update(cx, |state, cx| {
                if state.value() != value.as_str() {
                    state.set_value(value, window, cx);
                }
            });
        }

        let body: gpui::AnyElement = if !data.graph_ready {
            kit::empty_state(
                "Material node catalog not published yet.",
                Some("waiting for the project-host node type catalog".to_owned()),
                &theme,
            )
            .into_any_element()
        } else if data.palette.total_count == 0 {
            kit::empty_state(
                "No material nodes in the node catalog.",
                Some("az-material-nodes registers the Material node library".to_owned()),
                &theme,
            )
            .into_any_element()
        } else {
            render_material_palette_list(&data.palette, &theme)
        };

        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(
                kit::panel_toolbar(&theme)
                    .child(kit::panel_title("Node Palette", &theme))
                    .child(kit::panel_subtitle(
                        format!("{} nodes", data.palette.total_count),
                        &theme,
                    )),
            )
            .child(
                kit::search_row(&theme).child(
                    Input::new(&self.filter_input)
                        .small()
                        .appearance(false)
                        .bordered(false)
                        .focus_bordered(false),
                ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .p_2p5()
                    .gap_1()
                    .child(body),
            )
            .into_any_element()
    }
}

/// The material palette list: a hint when no graph is open, a no-match hint
/// when the filter excludes everything, then one section per node category.
fn render_material_palette_list(
    palette: &MaterialPaletteProjection,
    theme: &Theme,
) -> gpui::AnyElement {
    let target = palette.target.clone();
    v_flex()
        .gap_2()
        .when(target.is_none(), |this| {
            this.child(kit::hint_text(
                "Open a material graph (.azmat.ron) to add nodes.",
                theme,
            ))
        })
        .when(
            palette.categories.is_empty() && !palette.filter.is_empty(),
            |this| {
                this.child(kit::hint_text(
                    format!("No material nodes match \"{}\".", palette.filter),
                    theme,
                ))
            },
        )
        .children(palette.categories.iter().map(|category| {
            let tint = palette_category_tint(&category.name, theme);
            let target = target.clone();
            v_flex()
                .gap_0p5()
                .child(kit::section_label(category.name.clone(), theme))
                .children(category.items.iter().map(move |item| {
                    let row =
                        kit::list_row(element_id("mat-palette", &item.node_type), theme, false)
                            .child(kit::status_dot(tint))
                            .child(kit::row_name(item.label.clone(), theme))
                            .child(kit::meta_text(item.port_summary.clone(), theme));
                    match target.clone() {
                        Some(target) => {
                            let node_type = item.node_type.clone();
                            let node_type_version = item.version;
                            row.on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                window.dispatch_action(
                                    Box::new(crate::actions::AddGraphNode {
                                        node_type: node_type.clone(),
                                        node_type_version,
                                        x: target.next_x,
                                        y: target.next_y,
                                    }),
                                    cx,
                                );
                            })
                        }
                        None => row.opacity(0.55).cursor_default(),
                    }
                }))
        }))
        .into_any_element()
}

impl Focusable for MaterialPalettePanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for MaterialPalettePanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(None, "Palette", kit::TabTone::Default)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for MaterialPalettePanel {}

// ---------------------------------------------------------------------------
// Right rail: preview + parameters
// ---------------------------------------------------------------------------

fn render_material_params(cx: &App) -> impl IntoElement {
    let theme = cx.theme().clone();
    let data = materials(cx);

    kit::panel_shell(
        "Material",
        Some("preview · parameters".to_owned()),
        v_flex()
            .gap_3()
            .child(render_preview_section(&data, &theme))
            .child(render_params_section(&data, &theme)),
        &theme,
    )
}

fn render_preview_section(data: &EditorMaterialsProjection, theme: &Theme) -> impl IntoElement {
    let shape = data.preview.shape;
    v_flex()
        .gap_1p5()
        .child(kit::section_label("Preview", theme))
        .child(
            div()
                .h(px(150.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(gpui::linear_gradient(
                    160.0,
                    gpui::linear_color_stop(theme.sidebar, 0.0),
                    gpui::linear_color_stop(theme.background, 1.0),
                ))
                .flex()
                .items_center()
                .justify_center()
                .child(data.preview.material.as_ref().map_or_else(
                    || {
                        div()
                            .text_size(px(11.0))
                            .text_color(theme.muted_foreground)
                            .child("No material selected")
                            .into_any_element()
                    },
                    |material| render_preview_swatch(material, shape).into_any_element(),
                )),
        )
        .child(
            h_flex()
                .gap_1()
                .children(MaterialPreviewShape::ALL.into_iter().map(|option| {
                    let selected = option == shape;
                    kit::option_chip(selected, theme)
                        .id(element_id("mat-preview-shape", option.label()))
                        .cursor_pointer()
                        .child(option.label())
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            window.dispatch_action(
                                Box::new(crate::actions::SetMaterialPreviewShape { shape: option }),
                                cx,
                            );
                        })
                        .into_any_element()
                })),
        )
        .children(data.preview.material.as_ref().map(|material| {
            let mut parts = Vec::new();
            if !material.bound.is_empty() {
                parts.push(format!("from properties: {}", material.bound.join(", ")));
            }
            if !material.defaulted.is_empty() {
                parts.push(format!("defaults: {}", material.defaulted.join(", ")));
            }
            kit::hint_text(parts.join(" · "), theme)
        }))
}

/// B3 v1 preview swatch: a styled div shaded from authored property values.
/// The colors here are document data (the material's own base color), not
/// design chrome; geometry follows the shape-switcher state.
fn render_preview_swatch(
    material: &MaterialPreviewMaterialProjection,
    shape: MaterialPreviewShape,
) -> impl IntoElement {
    let base = hsla_from_rgba(material.base_color);
    let roughness = material.roughness.clamp(0.0, 1.0);
    let metallic = material.metallic.clamp(0.0, 1.0);

    // Tint: base color. Contrast: metallic deepens the shadow side and
    // brightens the lit side. Highlight: roughness widens and dims it.
    let lit = with_lightness(base, metallic.mul_add(0.12, base.l + 0.14));
    let shadow = with_lightness(base, base.l * metallic.mul_add(-0.45, 0.6));
    let highlight_alpha = roughness.mul_add(-0.6, 0.85);
    let highlight_size = roughness.mul_add(0.38, 0.16);

    let (width, height, radius) = match shape {
        MaterialPreviewShape::Sphere => (104.0_f32, 104.0_f32, 52.0_f32),
        MaterialPreviewShape::Cube => (94.0, 94.0, 10.0),
        MaterialPreviewShape::Plane => (132.0, 64.0, 6.0),
        MaterialPreviewShape::Mesh => (98.0, 98.0, 30.0),
    };
    let highlight_px = f32::min(width, height) * highlight_size;

    div()
        .w(px(width))
        .h(px(height))
        .rounded(px(radius))
        .overflow_hidden()
        .relative()
        .bg(gpui::linear_gradient(
            135.0,
            gpui::linear_color_stop(lit, 0.0),
            gpui::linear_color_stop(shadow, 1.0),
        ))
        .child(
            div()
                .absolute()
                .top(px(height * 0.16))
                .left(px(width * 0.2))
                .w(px(highlight_px))
                .h(px(highlight_px))
                .rounded_full()
                .bg(Hsla {
                    h: 0.0,
                    s: 0.0,
                    l: 1.0,
                    a: highlight_alpha.clamp(0.1, 0.9),
                }),
        )
}

fn render_params_section(data: &EditorMaterialsProjection, theme: &Theme) -> impl IntoElement {
    v_flex()
        .gap_1p5()
        .child(kit::section_label("Material Parameters", theme))
        .child(match data.params.as_ref() {
            None => kit::hint_text(
                "Select a material (.azmaterial.ron) or material type source to inspect its \
                 parameters.",
                theme,
            )
            .into_any_element(),
            Some(params) if params.rows.is_empty() => v_flex()
                .gap_1()
                .child(kit::status_row("Document", params.label.clone(), theme))
                .child(kit::hint_text(
                    "No parameters authored on this document.",
                    theme,
                ))
                .into_any_element(),
            Some(params) => v_flex()
                .gap_1()
                .child(kit::status_row(
                    params.schema_label.clone(),
                    params.label.clone(),
                    theme,
                ))
                .children(
                    params
                        .rows
                        .iter()
                        .map(|row| render_param_row(row, theme).into_any_element()),
                )
                .into_any_element(),
        })
}

fn render_param_row(row: &MaterialParamRowProjection, theme: &Theme) -> impl IntoElement {
    h_flex()
        .min_h(px(28.0))
        .items_center()
        .gap_2()
        .child(
            div()
                .w(relative(0.4))
                .flex_none()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(row.label.clone()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(render_param_control(&row.control, theme)),
        )
}

fn render_param_control(
    control: &MaterialParamControlProjection,
    theme: &Theme,
) -> gpui::AnyElement {
    match control {
        MaterialParamControlProjection::Slider { fraction, display } => {
            render_param_slider(*fraction, display, theme)
        }
        MaterialParamControlProjection::Color { rgba, display } => {
            render_param_color(*rgba, display, theme)
        }
        MaterialParamControlProjection::Texture { path } => h_flex()
            .items_center()
            .gap_1p5()
            .h(px(23.0))
            .px_1p5()
            .rounded(px(4.0))
            .bg(theme.input_background())
            .border_1()
            .border_color(theme.border)
            .child(
                Icon::new(IconName::File)
                    .with_size(px(13.0))
                    .text_color(theme.success),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_size(px(10.5))
                    .text_color(theme.foreground)
                    .child(path.clone()),
            )
            .into_any_element(),
        MaterialParamControlProjection::Toggle { value } => h_flex()
            .items_center()
            .gap_1p5()
            .child(kit::status_dot(if *value {
                theme.success
            } else {
                theme.muted_foreground
            }))
            .child(
                div()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(10.5))
                    .text_color(theme.foreground)
                    .child(if *value { "true" } else { "false" }),
            )
            .into_any_element(),
        MaterialParamControlProjection::Value { display } => div()
            .font_family(theme.mono_font_family.clone())
            .text_size(px(10.5))
            .text_color(theme.foreground)
            .overflow_hidden()
            .text_ellipsis()
            .whitespace_nowrap()
            .child(display.clone())
            .into_any_element(),
    }
}

/// Read-only slider control: a filled track plus the authored value.
fn render_param_slider(fraction: f32, display: &str, theme: &Theme) -> gpui::AnyElement {
    h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .h(px(4.0))
                .rounded(px(2.0))
                .bg(theme.muted)
                .child(
                    div()
                        .w(relative(fraction.clamp(0.0, 1.0)))
                        .h_full()
                        .rounded(px(2.0))
                        .bg(theme.accent),
                ),
        )
        .child(
            div()
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.5))
                .text_color(theme.foreground)
                .child(display.to_owned()),
        )
        .into_any_element()
}

/// Read-only color control: the authored swatch plus its literal text.
fn render_param_color(rgba: [f32; 4], display: &str, theme: &Theme) -> gpui::AnyElement {
    h_flex()
        .items_center()
        .gap_1p5()
        .h(px(23.0))
        .px_1()
        .rounded(px(4.0))
        .bg(theme.input_background())
        .border_1()
        .border_color(theme.border)
        .child(
            // Swatch color is authored document data, not chrome.
            div()
                .flex_none()
                .size(px(14.0))
                .rounded(px(3.0))
                .border_1()
                .border_color(theme.border)
                .bg(hsla_from_rgba(rgba)),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.5))
                .text_color(theme.muted_foreground)
                .child(display.to_owned()),
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// Center: material graph workbench
// ---------------------------------------------------------------------------

/// Center workbench: material document tabs + Save over the shared visual
/// graph canvas.
///
/// The canvas is the existing [`VisualGraphPanel`] hosting the graph
/// controller's current document; the materials controller routes
/// `.azmat.ron` selections into that controller.
///
/// No Apply button: applying a compiled material to a running scene has no
/// real path yet — the material graph→shader codegen behind
/// `azoth.material.shader-pipeline` is the explicit TODO in az-material-nodes,
/// so an Apply control would be a dead button. Save (graph document save) is
/// the real persistence action.
pub struct MaterialWorkbenchPanel {
    focus_handle: FocusHandle,
    graph_canvas: Entity<VisualGraphPanel>,
}

impl MaterialWorkbenchPanel {
    pub const NAME: &'static str = "material-workbench";

    pub fn init(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let graph_canvas = cx.new(|cx| VisualGraphPanel::init(window, cx));
        Self {
            focus_handle: cx.focus_handle(),
            graph_canvas,
        }
    }
}

impl Render for MaterialWorkbenchPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        if let Some(placeholder) =
            crate::panels::render_project_host_connection_placeholder("Material", cx)
        {
            return placeholder;
        }
        let theme = cx.theme().clone();
        let data = materials(cx);
        let hosts_graph = data.canvas.material_graph_document_id.is_some();

        let toolbar = kit::panel_toolbar(&theme)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_1()
                    .overflow_hidden()
                    .when(data.documents.is_empty(), |this| {
                        this.child(kit::panel_subtitle("no material sources", &theme))
                    })
                    .children(data.documents.iter().map(|document| {
                        let label = if document.unsaved_changes {
                            format!("{}*", document.label)
                        } else {
                            document.label.clone()
                        };
                        let document_id = document.document_id.clone();
                        kit::filter_chip(
                            element_id("mat-doc-tab", &document.document_id),
                            label,
                            None,
                            None,
                            Some(document.kind.label().to_owned()),
                            document.selected,
                            &theme,
                        )
                        .on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            window.dispatch_action(
                                Box::new(crate::actions::SelectMaterialDocument {
                                    document_id: document_id.clone(),
                                }),
                                cx,
                            );
                        })
                    })),
            )
            .when(hosts_graph, |this| {
                this.child(kit::toolbar_icon_button(
                    "mat-save-graph",
                    IconName::Save,
                    "Save material graph",
                    crate::actions::SaveGraphDocument,
                    &theme,
                ))
            });

        let body: gpui::AnyElement = if hosts_graph {
            div()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .child(self.graph_canvas.clone())
                .into_any_element()
        } else {
            kit::empty_state(
                data.canvas.empty_reason,
                Some(
                    "materials/graphs/*.azmat.ron · the palette adds nodes to the open graph"
                        .to_owned(),
                ),
                &theme,
            )
            .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(theme.background)
            .child(toolbar)
            .child(body)
            .into_any_element()
    }
}

impl Focusable for MaterialWorkbenchPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for MaterialWorkbenchPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<'_, Self>) -> impl IntoElement {
        kit::tab_title(Some("gradient"), "Material", kit::TabTone::Warning)
    }

    fn inner_padding(&self, _cx: &App) -> bool {
        false
    }
}

impl gpui::EventEmitter<gpui_component::dock::PanelEvent> for MaterialWorkbenchPanel {}

mode_panel!(
    MaterialParamsPanel,
    "material-params",
    "Params",
    render_material_params,
    None,
    kit::TabTone::Default
);
