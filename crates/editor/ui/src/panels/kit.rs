//! Editor panel design kit.
//!
//! Shared, theme-driven building blocks that give every dock panel the dense,
//! flat, engine-tool look defined by the Azoth editor design system
//! (`DESIGN.md` / `design/aether-editor.html`). Panels compose these instead
//! of hand-styling chrome, so spacing, heights, hover states, and typography
//! stay consistent across the editor.
//!
//! Primitives intentionally return plain `Div` / `Stateful<Div>` builders so
//! callers keep full control over children and event wiring — the kit owns the
//! *look*, the panel owns the *behaviour*.

use gpui::{
    App, Div, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, Stateful, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};
use gpui_component::scroll::ScrollableElement;
use gpui_component::theme::Theme;
use gpui_component::tooltip::Tooltip;
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, StyledExt, h_flex, v_flex};

/// Height of a panel header / sub-toolbar strip.
pub const TOOLBAR_H: f32 = 32.0;
/// Height of an inline search / breadcrumb row.
pub const SEARCH_H: f32 = 30.0;
/// Height of a dense tree / list row.
pub const ROW_H: f32 = 24.0;
/// Height of a panel footer strip.
pub const FOOTER_H: f32 = 24.0;

// ---------------------------------------------------------------------------
// Layout arithmetic
//
// Panels lay out in `f32`, but the quantities they lay out from are integers:
// a tree depth, a list length, a millisecond position inside a timeline. The
// four helpers below are the only place those widen, so the bound that makes
// the widening exact is stated once instead of at every call site.
// ---------------------------------------------------------------------------

/// Widen a rendered count — a tree depth, a list length, a cell index — for
/// layout math.
///
/// `f32` represents every integer below 2^24 exactly. A count that reaches
/// sixteen million has no rendered representation (nothing lays out sixteen
/// million rows, and no authored tree is that deep), so the lossy range is
/// unreachable from anything a panel can display.
#[must_use]
pub const fn count(value: usize) -> f32 {
    // Rust offers no lossless usize -> f32 conversion; the bound above keeps
    // every caller inside f32's exactly-representable integer range.
    #[allow(clippy::cast_precision_loss)]
    {
        value as f32
    }
}

/// `numerator / denominator` as a layout ratio, `0.0` when the denominator is
/// zero.
///
/// Both sides are rendered quantities — a millisecond offset inside a
/// timeline, a tick index inside a ruler — and stay below `f32`'s 2^24
/// exactly-representable ceiling, which is over four hours in milliseconds.
#[must_use]
pub fn ratio(numerator: u32, denominator: u32) -> f32 {
    if denominator == 0 {
        return 0.0;
    }
    // See `count`: both operands are bounded by what a panel can display.
    #[allow(clippy::cast_precision_loss)]
    {
        numerator as f32 / denominator as f32
    }
}

/// Project a `0.0..=1.0` fraction back across `span`, rounded to the nearest
/// whole unit. The inverse of [`ratio`].
///
/// The fraction is clamped before the multiply and the result is clamped to
/// `span` after it, so the narrowing back to `u32` can neither overflow nor
/// see a negative — and float-to-integer `as` saturates rather than wrapping
/// even if it could.
#[must_use]
pub fn scaled(fraction: f32, span: u32) -> u32 {
    // No checked f32 -> u32 conversion exists; the clamps here bound the
    // product to 0.0..=span before it narrows.
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    {
        ((span as f32 * fraction.clamp(0.0, 1.0)).round() as u32).min(span)
    }
}

/// Left padding for a tree row at `depth`: `base`, plus `step` per level.
///
/// Takes any integer depth the panel projections carry (`u32` on the wire
/// shapes, `usize` on the locally-built trees); a depth that would not fit a
/// `usize` is not a depth anything rendered.
#[must_use]
pub fn indent(depth: impl TryInto<usize>, step: f32, base: f32) -> gpui::Pixels {
    px(count(depth.try_into().unwrap_or(usize::MAX)).mul_add(step, base))
}

/// Theme-driven tint for a dock tab's Material Symbol icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TabTone {
    /// Inherit the tab's active/inactive foreground color.
    #[default]
    Default,
    Muted,
    Accent,
    Success,
    Warning,
    Danger,
}

impl TabTone {
    fn color(self, theme: &Theme) -> Option<Hsla> {
        match self {
            Self::Default => None,
            Self::Muted => Some(theme.muted_foreground),
            Self::Accent => Some(theme.accent),
            Self::Success => Some(theme.success),
            Self::Warning => Some(theme.warning),
            Self::Danger => Some(theme.danger),
        }
    }
}

#[derive(IntoElement)]
struct TabTitle {
    icon: Option<&'static str>,
    label: SharedString,
    tone: TabTone,
}

impl RenderOnce for TabTitle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .h_full()
            .items_center()
            .gap(px(6.0))
            .text_size(px(11.5))
            .when_some(self.icon, |this, symbol| {
                let icon = Icon::empty()
                    .path(material_symbol_path(symbol))
                    .with_size(px(15.0));
                this.child(match self.tone.color(theme) {
                    Some(color) => icon.text_color(color),
                    None => icon,
                })
            })
            .child(self.label)
    }
}

/// Build the icon + label element used as a panel's dock-tab title.
///
/// `icon` is a Material Symbols name. Pass `None` when the design does not
/// show a glyph for the panel. Colors always resolve through [`TabTone`].
#[must_use]
pub fn tab_title(
    icon: Option<&'static str>,
    label: impl Into<SharedString>,
    tone: TabTone,
) -> impl IntoElement {
    TabTitle {
        icon,
        label: label.into(),
        tone,
    }
}

fn material_symbol_path(symbol: &str) -> SharedString {
    match symbol {
        "check_box_outline_blank" => "icons/dc/crop_square.svg".into(),
        "speed" => "icons/gauge.svg".into(),
        "terminal" => "icons/square-terminal.svg".into(),
        _ => format!("icons/dc/{symbol}.svg").into(),
    }
}

/// Render a theme-tinted Material Symbol from the editor's checked-in SVG set.
///
/// Inspector cards and asset controls use schema-declared symbol names, so the
/// kit owns the path convention instead of duplicating it in each panel.
#[must_use]
pub fn material_symbol_icon(symbol: impl AsRef<str>, size: f32, color: Hsla) -> Icon {
    Icon::empty()
        .path(material_symbol_path(symbol.as_ref()))
        .with_size(px(size))
        .text_color(color)
}

/// Axis identity used by compact vector controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InspectorAxis {
    X,
    Y,
    Z,
    W,
}

impl InspectorAxis {
    const fn label(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::W => "W",
        }
    }

    fn color(self, theme: &Theme) -> Hsla {
        match self {
            Self::X => theme.danger,
            Self::Y => theme.success,
            Self::Z => theme.accent,
            Self::W => theme.warning,
        }
    }
}

/// Dense recessed inspector control chrome (23px, bordered, theme-driven).
#[must_use]
pub fn inspector_control(theme: &Theme) -> Div {
    h_flex()
        .w_full()
        .h(px(23.0))
        .min_w_0()
        .items_center()
        .rounded(px(4.0))
        .bg(theme.input_background())
        .border_1()
        .border_color(theme.border)
}

/// One compact vector input with a colored X/Y/Z/W identity chip.
#[must_use]
pub fn inspector_axis_control(axis: InspectorAxis, child: impl IntoElement, theme: &Theme) -> Div {
    let tint = axis.color(theme);
    inspector_control(theme)
        .flex_1()
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .w(px(15.0))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .bg(tint.opacity(0.78))
                .font_family(theme.mono_font_family.clone())
                .text_size(px(9.5))
                .font_semibold()
                .text_color(theme.accent_foreground)
                .child(axis.label()),
        )
        .child(div().flex_1().min_w_0().px_1().child(child))
}

/// Dot-style pill toggle used by component and boolean controls.
///
/// The caller owns event wiring. Disabled toggles remain visibly present but
/// muted so unsupported authored fields never masquerade as interactive UI.
#[must_use]
pub fn inspector_toggle(
    id: impl Into<ElementId>,
    checked: bool,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let track = if checked { theme.accent } else { theme.border };
    let knob = if checked {
        theme.accent_foreground
    } else {
        theme.muted_foreground
    };
    h_flex()
        .id(id)
        .w(px(25.0))
        .h(px(14.0))
        .flex_none()
        .items_center()
        .rounded_full()
        .p(px(2.0))
        .bg(track.opacity(if disabled { 0.38 } else { 1.0 }))
        .when(checked, gpui::Styled::justify_end)
        .when(!checked, gpui::Styled::justify_start)
        .when(!disabled, gpui::Styled::cursor_pointer)
        .child(
            div()
                .size(px(10.0))
                .rounded_full()
                .bg(knob.opacity(if disabled { 0.42 } else { 1.0 })),
        )
}

/// Disabled/selectable-looking metadata box used by Tag and Layer.
#[must_use]
pub fn inspector_metadata_select(value: impl Into<String>, disabled: bool, theme: &Theme) -> Div {
    inspector_control(theme)
        .px_1p5()
        .gap_1()
        .text_size(px(10.5))
        .text_color(if disabled {
            theme.muted_foreground.opacity(0.55)
        } else {
            theme.foreground
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(value.into()),
        )
        .child(
            material_symbol_icon("arrow_drop_down", 14.0, theme.muted_foreground)
                .opacity(if disabled { 0.35 } else { 1.0 }),
        )
}

/// A panel header / sub-toolbar strip: fixed height, tab-bar tone, bottom
/// border. Caller appends a title and trailing action buttons.
#[must_use]
pub fn panel_toolbar(theme: &Theme) -> Div {
    h_flex()
        .w_full()
        .h(px(TOOLBAR_H))
        .flex_none()
        .items_center()
        .gap_1()
        .px_2()
        .bg(theme.tab_bar)
        .border_b_1()
        .border_color(theme.border)
}

/// A panel title used inside [`panel_toolbar`].
#[must_use]
pub fn panel_title(text: impl Into<String>, theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .text_size(px(13.0))
        .font_semibold()
        .text_color(theme.foreground)
        .child(text.into())
}

/// A small uppercase section label (e.g. "Scene Layers", "Preview").
#[must_use]
pub fn section_label(text: impl Into<String>, theme: &Theme) -> Div {
    div()
        .text_size(px(10.0))
        .font_medium()
        .text_color(theme.muted_foreground)
        .child(uppercase(text))
}

/// An inline search row: fixed height, sidebar tone, bottom border, a leading
/// search glyph. Caller appends the `Input` (and any trailing filter glyph).
#[must_use]
pub fn search_row(theme: &Theme) -> Div {
    h_flex()
        .w_full()
        .h(px(SEARCH_H))
        .flex_none()
        .items_center()
        .gap_1p5()
        .px_2()
        .bg(theme.sidebar)
        .border_b_1()
        .border_color(theme.border)
        .child(
            Icon::new(IconName::Search)
                .with_size(px(15.0))
                .text_color(theme.muted_foreground),
        )
}

/// A dense, flat tree / list row. Selected rows get the active list tone and an
/// accent left edge; unselected rows get a subtle hover.
#[must_use]
pub fn list_row(id: impl Into<ElementId>, theme: &Theme, selected: bool) -> Stateful<Div> {
    let row = h_flex()
        .id(id)
        .w_full()
        .h(px(ROW_H))
        .items_center()
        .gap_1p5()
        .pr_2()
        .text_color(theme.foreground)
        .cursor_pointer();
    if selected {
        row.bg(theme.list_active)
            .border_l_2()
            .border_color(theme.list_active_border)
    } else {
        row.border_l_2()
            .border_color(theme.transparent)
            .hover(|this| this.bg(theme.list_hover))
    }
}

/// A disclosure caret for a tree row. `None` renders an inert spacer so rows
/// without children stay column-aligned with rows that have one.
#[must_use]
pub fn row_caret(expanded: Option<bool>, theme: &Theme) -> Div {
    let slot = div().flex_none().w(px(16.0)).flex().justify_center();
    match expanded {
        Some(open) => slot.child(material_symbol_icon(
            if open {
                "arrow_drop_down"
            } else {
                "chevron_right"
            },
            16.0,
            theme.muted_foreground,
        )),
        None => slot,
    }
}

/// A leading row glyph in a caller-chosen tint.
#[must_use]
pub fn row_icon(icon: IconName, color: impl Into<Hsla>) -> Icon {
    Icon::new(icon).with_size(px(15.0)).text_color(color)
}

/// The flexible name cell of a row (ellipsised, single line).
#[must_use]
pub fn row_name(text: impl Into<String>, theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .text_size(px(11.5))
        .text_color(theme.foreground)
        .child(text.into())
}

/// A small accent-tinted chip used for inline tags (layer, kind, status).
#[must_use]
pub fn tag_chip(text: impl Into<String>, tint: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(3.0))
        .bg(tint.opacity(0.16))
        .text_size(px(9.0))
        .font_semibold()
        .text_color(tint)
        .child(uppercase(text))
}

/// A trailing monospace metadata cell (counts, sizes, revisions).
#[must_use]
pub fn meta_text(text: impl Into<String>, theme: &Theme) -> Div {
    div()
        .flex_none()
        .font_family("monospace")
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(text.into())
}

/// A small status dot.
#[must_use]
pub fn status_dot(color: Hsla) -> Div {
    div().flex_none().size(px(6.0)).rounded_full().bg(color)
}

/// A minimal status dot whose explanation stays out of the row until hover.
#[must_use]
pub fn status_dot_with_tooltip(
    id: impl Into<ElementId>,
    color: Hsla,
    tooltip: impl Into<SharedString>,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(6.0))
        .rounded_full()
        .bg(color)
        .tooltip({
            let tooltip = tooltip.into();
            move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx)
        })
}

/// A panel footer strip: fixed height, tab-bar tone, top border, monospace.
/// Caller appends summary cells.
#[must_use]
pub fn count_footer(theme: &Theme) -> Div {
    h_flex()
        .w_full()
        .h(px(FOOTER_H))
        .flex_none()
        .items_center()
        .gap_1p5()
        .px_2p5()
        .border_t_1()
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .font_family("monospace")
        .text_size(px(10.5))
        .text_color(theme.muted_foreground)
}

/// A centered empty / placeholder state for a panel body.
#[must_use]
pub fn empty_state(message: impl Into<String>, sub: Option<String>, theme: &Theme) -> Div {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_1()
        .p_4()
        .child(
            div()
                .text_size(px(12.0))
                .text_color(theme.muted_foreground)
                .child(message.into()),
        )
        .when_some(sub, |this, sub| {
            this.child(
                div()
                    .font_family("monospace")
                    .text_size(px(10.5))
                    .text_color(theme.muted_foreground.opacity(0.7))
                    .child(sub),
            )
        })
}

/// A 24px ghost icon button for panel toolbars that dispatches an action.
#[must_use]
pub fn toolbar_icon_button<A>(
    id: impl Into<ElementId>,
    icon: IconName,
    tooltip: &'static str,
    action: A,
    theme: &Theme,
) -> Stateful<Div>
where
    A: gpui::Action + Clone + 'static,
{
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .size(px(24.0))
        .rounded(px(4.0))
        .text_color(theme.muted_foreground)
        .hover(|this| this.bg(theme.secondary_hover).text_color(theme.foreground))
        .cursor_pointer()
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(Icon::new(icon).with_size(px(16.0)))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            window.dispatch_action(Box::new(action.clone()), cx);
        })
}

/// A pill toggle/filter chip (segmented-control member) with an optional
/// leading icon and trailing count. Active chips read as primary.
#[must_use]
pub fn filter_chip(
    id: impl Into<ElementId>,
    label: impl Into<String>,
    icon: Option<IconName>,
    icon_color: Option<Hsla>,
    count: Option<String>,
    active: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let base = h_flex()
        .id(id)
        .flex_none()
        .h(px(22.0))
        .items_center()
        .gap_1()
        .px_2()
        .rounded(px(5.0))
        .text_size(px(11.0))
        .cursor_pointer();
    let base = if active {
        base.bg(theme.secondary).text_color(theme.foreground)
    } else {
        base.text_color(theme.muted_foreground)
            .hover(|this| this.text_color(theme.foreground))
    };
    base.when_some(icon, |this, icon| {
        this.child(
            Icon::new(icon)
                .with_size(px(14.0))
                .text_color(icon_color.unwrap_or(theme.muted_foreground)),
        )
    })
    .child(label.into())
    .when_some(count, |this, count| {
        this.child(
            div()
                .font_family("monospace")
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .child(count),
        )
    })
}

/// Wrap an `Input` in the recessed, bordered field chrome used across the
/// inspector and toolbars. Caller passes the already-built input element.
#[must_use]
pub fn input_field(child: impl IntoElement, theme: &Theme) -> Div {
    h_flex()
        .w_full()
        .h(px(23.0))
        .items_center()
        .px_1p5()
        .rounded(px(4.0))
        .bg(theme.input_background())
        .border_1()
        .border_color(theme.border)
        .hover(|this| this.border_color(theme.ring.opacity(0.6)))
        .child(child)
}

/// Recessed, bordered chrome for a small inline control button used in the
/// inspector (steppers, `+`/`-`, toggles, clear). Caller adds id/child/onclick.
#[must_use]
pub fn field_button(theme: &Theme) -> Div {
    div()
        .px_2()
        .py(px(2.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_size(px(11.0))
        .text_color(theme.foreground)
}

/// Chrome for a selectable inline option chip (enum variants), styled by its
/// `selected` state. Caller adds id/child/onclick.
#[must_use]
pub fn option_chip(selected: bool, theme: &Theme) -> Div {
    let base = div()
        .px_2()
        .py(px(2.0))
        .rounded(px(4.0))
        .border_1()
        .text_size(px(11.0));
    if selected {
        base.border_color(theme.accent)
            .bg(theme.list_active)
            .text_color(theme.foreground)
    } else {
        base.border_color(theme.border)
            .bg(theme.secondary)
            .text_color(theme.muted_foreground)
    }
}

/// A frosted, bordered overlay container floated over the viewport canvas
/// (HUD stats, view pills, nav info). Monospace, dim, theme-driven — replaces
/// hand-picked translucent colours.
#[must_use]
pub fn viewport_overlay(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(2.0))
        .px_2()
        .py_1p5()
        .rounded(px(6.0))
        .bg(theme.background.opacity(0.82))
        .border_1()
        .border_color(theme.border)
        .font_family("monospace")
        .text_size(px(10.5))
        .text_color(theme.muted_foreground)
}

/// A dim trailing caption following a [`panel_title`] in a toolbar header
/// (e.g. "palette left · params right").
#[must_use]
pub fn panel_subtitle(text: impl Into<String>, theme: &Theme) -> Div {
    div()
        .flex_none()
        .text_size(px(10.5))
        .text_color(theme.muted_foreground)
        .child(text.into())
}

/// A single-line muted caption used inline within a panel/workbench body —
/// distinct from [`empty_state`], which centers and fills the whole area.
#[must_use]
pub fn hint_text(text: impl Into<String>, theme: &Theme) -> Div {
    div()
        .text_size(px(11.0))
        .text_color(theme.muted_foreground)
        .child(text.into())
}

/// A label/value metadata row used in workbench status summaries.
#[must_use]
pub fn status_row(label: impl Into<String>, value: impl Into<String>, theme: &Theme) -> Div {
    h_flex()
        .gap_2()
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.0))
                .text_color(theme.foreground)
                .child(value.into()),
        )
}

/// A dock-panel shell: a [`panel_toolbar`] header (title + optional
/// subtitle) over a padded, vertically-scrollable sidebar-toned body.
///
/// Used by the per-mode navigator/detail side panels (materials palette,
/// script files, game-data catalog, animation navigator, ...).
#[must_use]
pub fn panel_shell(
    title: impl Into<String>,
    subtitle: Option<String>,
    body: impl IntoElement,
    theme: &Theme,
) -> Div {
    v_flex()
        .size_full()
        .min_w_0()
        .min_h_0()
        .bg(theme.sidebar)
        .child(
            panel_toolbar(theme)
                .child(panel_title(title, theme))
                .children(subtitle.map(|text| panel_subtitle(text, theme))),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_2p5()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .child(body),
                ),
        )
}

/// A center-workbench shell: a [`panel_toolbar`] header (title + subtitle)
/// over a padded content area.
///
/// Used by the per-mode document workbenches (material graph, script editor,
/// data table, sequencer, animation).
#[must_use]
pub fn workbench_shell(
    title: impl Into<String>,
    subtitle: impl Into<String>,
    body: impl IntoElement,
    theme: &Theme,
) -> Div {
    v_flex()
        .size_full()
        .min_w_0()
        .min_h_0()
        .bg(theme.background)
        .child(
            panel_toolbar(theme)
                .child(panel_title(title, theme))
                .child(panel_subtitle(subtitle, theme)),
        )
        .child(
            v_flex().flex_1().min_w_0().min_h_0().p_4().gap_2p5().child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(body),
            ),
        )
}

fn uppercase(text: impl Into<String>) -> String {
    text.into().to_uppercase()
}
