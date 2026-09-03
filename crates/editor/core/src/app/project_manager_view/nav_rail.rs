use super::super::{chrome_icon, project_manager::ProjectManagerView};
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Rems,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{h_flex, theme::Theme, v_flex};

pub(super) fn render_nav_rail_region(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let version = env!("CARGO_PKG_VERSION");
    let mut rail = v_flex()
        .flex_none()
        .w(px(212.0))
        .h_full()
        .p(px(10.0))
        .gap(px(2.0))
        .bg(theme.list_even)
        .border_r_1()
        .border_color(theme.border);

    for item in view.nav_items() {
        let foreground = if item.active {
            theme.tab_active_foreground
        } else {
            theme.sidebar_foreground
        };
        let action = item.clone();
        rail = rail.child(
            h_flex()
                .id(format!("manager-nav-{}", item.label))
                .items_center()
                .gap(px(10.0))
                .h(px(36.0))
                .px(px(11.0))
                .rounded(px(7.0))
                .text_size(Rems(0.78))
                .text_color(foreground)
                .when(item.active, |this| {
                    this.bg(theme.sidebar_accent)
                        .border_l_2()
                        .border_color(theme.accent)
                })
                .when(!item.active, |this| {
                    this.hover(|this| this.bg(theme.secondary))
                })
                .child(chrome_icon(item.icon, 18.0, foreground))
                .child(item.label)
                .on_click(cx.listener(move |_this, event, window, cx| {
                    action.on_click(event, window, cx);
                })),
        );
    }

    rail.child(div().flex_1())
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .pt(px(10.0))
                .px(px(9.0))
                .border_t_1()
                .border_color(theme.table_row_border)
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(Rems(0.64))
                        .text_color(theme.muted_foreground)
                        .child(format!("AZoth {version}")),
                ),
        )
        .into_any_element()
}
