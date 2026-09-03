use super::super::project_manager::ProjectManagerView;
use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, px};
use gpui_component::{Icon, StyledExt as _, h_flex, theme::Theme, v_flex};

pub(super) fn render_summary_region(view: &ProjectManagerView, theme: &Theme) -> AnyElement {
    v_flex()
        .flex_none()
        .w(px(248.0))
        .h_full()
        .p(px(18.0))
        .gap(px(14.0))
        .bg(theme.sidebar)
        .border_l_1()
        .border_color(theme.border)
        .child(
            gpui::div()
                .text_size(px(12.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child("Project summary"),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(9.0))
                .child(Icon::new(view.sum_icon()).size(px(20.0)))
                .child(
                    v_flex()
                        .gap(px(2.0))
                        .child(
                            gpui::div()
                                .text_size(px(12.0))
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child(view.name()),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(10.0))
                                .text_color(theme.muted_foreground)
                                .child(format!("{} template", view.sum_template())),
                        ),
                ),
        )
        .child(
            v_flex()
                .gap(px(7.0))
                .children(view.sum_rows().into_iter().map(|row| {
                    h_flex()
                        .justify_between()
                        .gap(px(12.0))
                        .text_size(px(10.5))
                        .child(
                            gpui::div()
                                .text_color(theme.muted_foreground)
                                .child(row.label),
                        )
                        .child(
                            gpui::div()
                                .text_color(theme.foreground)
                                .text_right()
                                .child(row.value),
                        )
                })),
        )
        .child(
            v_flex()
                .gap(px(7.0))
                .pt(px(4.0))
                .border_t_1()
                .border_color(theme.border)
                .child(
                    gpui::div()
                        .text_size(px(10.0))
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .child(format!("{} enabled gems", view.gem_count())),
                )
                .children(view.sum_gems().into_iter().map(|gem| {
                    h_flex()
                        .items_center()
                        .gap(px(7.0))
                        .text_size(px(10.5))
                        .text_color(theme.foreground)
                        .child(Icon::new(gem.icon).size(px(14.0)))
                        .child(gem.name)
                })),
        )
        .into_any_element()
}
