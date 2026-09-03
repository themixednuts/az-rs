//! Shared Aether presentation primitives used by multiple view regions.

use std::fmt;
use std::path::Path;

use gpui::{Hsla, Rgba};

use crate::app::aether_common::{AetherItem, AetherStyle};
pub(crate) fn set_item_style(item: &mut AetherItem, field: &str, style: AetherStyle) {
    item.style_fields.insert(field.to_owned(), style.clone());
    match field {
        "style" => item.style = style,
        "btnStyle" => item.btn_style = style,
        "badgeStyle" => item.badge_style = style,
        _ => {}
    }
}

pub(crate) fn aether_tab_item((key, label, icon): (&str, &str, &str)) -> AetherItem {
    AetherItem {
        key: key.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        ..AetherItem::default()
    }
}

pub(crate) fn toolbar_action_item(
    kind: &str,
    key: &str,
    label: &str,
    icon: &str,
    title: &str,
) -> AetherItem {
    AetherItem {
        kind: kind.to_owned(),
        key: key.to_owned(),
        label: label.to_owned(),
        icon: icon.to_owned(),
        title: title.to_owned(),
        ..AetherItem::default()
    }
}

pub(crate) fn bottom_tab_badge_style(theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("fontSize", "9px".to_owned()),
        ("fontWeight", "600".to_owned()),
        ("color", hsla_css(theme.warning_foreground)),
        ("background", hsla_css(theme.warning)),
        ("borderRadius", "8px".to_owned()),
        ("padding", "1px 5px".to_owned()),
        ("marginLeft", "1px".to_owned()),
    ])
}

pub(crate) fn toolbar_segment_style(with_left_border: bool, gap: u32) -> AetherStyle {
    let mut pairs = vec![
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", format!("{gap}px")),
        ("padding", "0 12px".to_owned()),
        ("cursor", "default".to_owned()),
    ];
    if with_left_border {
        pairs.push(("borderLeft", "1px solid #2c313a".to_owned()));
    }
    AetherStyle::from_pairs(&pairs)
}

pub(crate) fn toggle_track_style(on: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("width", "26px".to_owned()),
        ("height", "15px".to_owned()),
        ("borderRadius", "8px".to_owned()),
        (
            "background",
            if on { "#3160a8" } else { "#3a404a" }.to_owned(),
        ),
        ("position", "relative".to_owned()),
        ("flex", "0 0 auto".to_owned()),
        ("cursor", "default".to_owned()),
    ])
}

pub(crate) fn toggle_knob_style(on: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "absolute".to_owned()),
        ("top", "2px".to_owned()),
        ("left", "2px".to_owned()),
        ("width", "11px".to_owned()),
        ("height", "11px".to_owned()),
        ("borderRadius", "50%".to_owned()),
        ("background", "#fff".to_owned()),
        (
            "transform",
            if on {
                "translateX(11px)"
            } else {
                "translateX(0)"
            }
            .to_owned(),
        ),
    ])
}

pub(crate) fn themed_toggle_track_style(
    on: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("width", "26px".to_owned()),
        ("height", "15px".to_owned()),
        ("borderRadius", "8px".to_owned()),
        (
            "background",
            hsla_css(if on {
                theme.primary
            } else {
                theme.secondary_active
            }),
        ),
        ("position", "relative".to_owned()),
        ("flex", "0 0 auto".to_owned()),
        ("cursor", "default".to_owned()),
    ])
}

pub(crate) fn themed_toggle_knob_style(
    on: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("position", "absolute".to_owned()),
        ("top", "2px".to_owned()),
        ("left", if on { "13px" } else { "2px" }.to_owned()),
        ("width", "11px".to_owned()),
        ("height", "11px".to_owned()),
        ("borderRadius", "50%".to_owned()),
        (
            "background",
            hsla_css(if on {
                theme.primary_foreground
            } else {
                theme.muted_foreground
            }),
        ),
    ])
}

pub(crate) fn rgb_u32_css(color: u32) -> String {
    let r = ((color >> 16) & 0xff) as u8;
    let g = ((color >> 8) & 0xff) as u8;
    let b = (color & 0xff) as u8;
    format!("rgb({r},{g},{b})")
}

pub(crate) fn item_can_expand(item: &AetherItem, allow_empty_caret: bool) -> bool {
    !item.key.is_empty()
        && (item.has_children
            || !item.props.0.is_empty()
            || !item.rows.0.is_empty()
            || !item.items.0.is_empty()
            || (!item.caret.is_empty() || allow_empty_caret) && item.open)
}

pub(crate) fn apply_expandable_item_state(item: &mut AetherItem, open: bool) {
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

pub(crate) fn path_stem_label(path: &str) -> String {
    az_editor_ui::naming::display_name(path).into_owned()
}

pub(crate) use crate::app::aether_common::non_empty_string_or;
pub(crate) fn plural_count(count: impl fmt::Display, noun: &str) -> String {
    let count = count.to_string();
    if count == "1" {
        format!("{count} {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

pub(crate) fn format_count(count: impl fmt::Display) -> String {
    count.to_string()
}

pub(crate) fn hsla_css(color: Hsla) -> String {
    let rgba = Rgba::from(color);
    let r = (rgba.r * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (rgba.g * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (rgba.b * 255.0).round().clamp(0.0, 255.0) as u8;
    if rgba.a >= 1.0 {
        format!("rgb({r},{g},{b})")
    } else {
        format!("rgba({r},{g},{b},{:.3})", rgba.a.clamp(0.0, 1.0))
    }
}

fn muted_text_style() -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("fontSize", "9.5px".to_owned()),
        ("color", "#6b7280".to_owned()),
        ("display", "block".to_owned()),
        ("fontFamily", "'IBM Plex Mono',monospace".to_owned()),
    ])
}

pub(crate) fn settings_select_option_style(
    selected: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "space-between".to_owned()),
        ("height", "30px".to_owned()),
        ("padding", "0 10px".to_owned()),
        ("fontSize", "12px".to_owned()),
        ("cursor", "pointer".to_owned()),
        ("borderRadius", "5px".to_owned()),
        (
            "color",
            hsla_css(if selected {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            hsla_css(if selected {
                theme.accent.opacity(0.14)
            } else {
                theme.transparent
            }),
        ),
    ])
}
