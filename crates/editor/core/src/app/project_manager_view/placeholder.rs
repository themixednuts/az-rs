use super::super::project_manager::ProjectManagerView;
use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, px};
use gpui_component::{Icon, StyledExt as _, theme::Theme, v_flex};

pub(super) fn render_placeholder_region(view: &ProjectManagerView, theme: &Theme) -> AnyElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .items_center()
        .justify_center()
        .gap(px(12.0))
        .bg(theme.background)
        .text_color(theme.muted_foreground)
        .child(Icon::new(ProjectManagerView::misc_icon()).size(px(48.0)))
        .child(
            gpui::div()
                .text_size(px(14.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(view.misc_title()),
        )
        .child(gpui::div().text_size(px(11.5)).child(view.misc_sub()))
        .into_any_element()
}
