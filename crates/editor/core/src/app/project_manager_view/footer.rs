use super::super::project_manager::ProjectManagerView;
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, px,
};
use gpui_component::{Icon, IconName, StyledExt as _, h_flex, theme::Theme};

pub(super) fn render_footer_region(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let back = if view.can_back() {
        h_flex()
            .id("project-manager-back")
            .items_center()
            .gap(px(6.0))
            .h(px(32.0))
            .px(px(12.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(theme.border)
            .text_color(theme.foreground)
            .text_size(px(12.0))
            .cursor_pointer()
            .hover(|this| this.bg(theme.secondary))
            .on_click(cx.listener(|this, event, window, cx| {
                this.on_back(event, window, cx);
            }))
            .child(Icon::new(IconName::ArrowLeft).size(px(16.0)))
            .child("Back")
            .into_any_element()
    } else {
        gpui::div().into_any_element()
    };
    let next_label = if view.last_step() {
        "Create project"
    } else {
        "Continue"
    };

    h_flex()
        .flex_none()
        .items_center()
        .gap(px(10.0))
        .px(px(24.0))
        .py(px(14.0))
        .border_t_1()
        .border_color(theme.border)
        .child(
            gpui::div()
                .flex_1()
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(view.footer_hint()),
        )
        .child(back)
        .child(
            h_flex()
                .id("project-manager-cancel")
                .items_center()
                .h(px(32.0))
                .px(px(12.0))
                .rounded(px(6.0))
                .text_color(theme.muted_foreground)
                .text_size(px(12.0))
                .cursor_pointer()
                .hover(|this| this.text_color(theme.foreground))
                .on_click(cx.listener(|this, event, window, cx| {
                    this.go_recent(event, window, cx);
                }))
                .child("Cancel"),
        )
        .child(
            h_flex()
                .id("project-manager-next")
                .items_center()
                .gap(px(6.0))
                .h(px(32.0))
                .px(px(14.0))
                .rounded(px(6.0))
                .bg(theme.primary)
                .text_color(theme.primary_foreground)
                .text_size(px(12.0))
                .font_semibold()
                .cursor_pointer()
                .hover(|this| this.bg(theme.primary_hover))
                .on_click(cx.listener(|this, event, window, cx| {
                    this.on_next(event, window, cx);
                }))
                .child(next_label)
                .child(Icon::new(IconName::ArrowRight).size(px(16.0))),
        )
        .into_any_element()
}
