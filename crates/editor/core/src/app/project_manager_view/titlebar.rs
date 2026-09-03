use gpui::{AnyElement, IntoElement as _, ParentElement as _, Styled as _, px};
use gpui_component::{TitleBar, h_flex, theme::Theme};

pub(super) fn render_titlebar_region(theme: &Theme) -> AnyElement {
    TitleBar::new()
        .child(
            h_flex()
                .items_center()
                .child(
                    gpui::div()
                        .w(px(14.0))
                        .h(px(14.0))
                        .rounded(px(2.0))
                        .mr(px(11.0))
                        .bg(theme.accent),
                )
                .child(
                    gpui::div()
                        .text_size(px(12.5))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("Aether"),
                )
                .child(
                    gpui::div()
                        .text_size(px(12.0))
                        .text_color(theme.muted_foreground)
                        .ml(px(7.0))
                        .child("Project Manager"),
                ),
        )
        .into_any_element()
}
