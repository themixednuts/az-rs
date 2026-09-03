use super::super::{
    chrome_icon, chrome_tooltip,
    project_manager::{ManagerRecentLayout, ManagerRecentSort, ProjectManagerView, RecentItem},
};
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _, Rems,
    StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _, px,
};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{IconName, StyledExt as _, h_flex, theme::Theme, v_flex};

pub(super) fn render_recent_region(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let projects = view.recent_projects();
    let is_empty = projects.is_empty();
    let subtitle = if is_empty {
        "Stored locally".to_string()
    } else {
        format!("{} recent - stored locally", projects.len())
    };
    let content = if is_empty {
        render_empty_state(theme)
    } else {
        match view.recent_layout() {
            ManagerRecentLayout::List => render_project_list(projects, theme, cx),
            ManagerRecentLayout::Grid => render_project_grid(projects, theme, cx),
        }
    };
    let controls = if is_empty {
        div().into_any_element()
    } else {
        h_flex()
            .flex_none()
            .items_center()
            .gap(px(10.0))
            .px(px(24.0))
            .pb(px(12.0))
            .text_size(Rems(0.7))
            .text_color(theme.muted_foreground)
            .child(render_sort_chip(view.recent_sort(), theme, cx))
            .child(div().flex_1())
            .child(render_layout_toggle(view.recent_layout(), theme, cx))
            .into_any_element()
    };

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(theme.background)
        .child(
            h_flex()
                .flex_none()
                .items_center()
                .gap_3()
                .px(px(24.0))
                .pt(px(20.0))
                .pb(px(14.0))
                .child(
                    v_flex()
                        .gap(px(2.0))
                        .child(
                            div()
                                .text_size(Rems(1.18))
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child("Projects"),
                        )
                        .child(
                            div()
                                .text_size(Rems(0.72))
                                .text_color(theme.muted_foreground)
                                .child(subtitle),
                        ),
                )
                .child(div().flex_1())
                .child(render_open_button("manager-open", theme, cx))
                .child(render_new_project_button(theme, cx)),
        )
        .child(controls)
        .child(content)
        .into_any_element()
}

fn render_new_project_button(theme: &Theme, cx: &Context<'_, ProjectManagerView>) -> AnyElement {
    h_flex()
        .id("manager-new")
        .items_center()
        .gap(px(7.0))
        .h(px(32.0))
        .px(px(14.0))
        .rounded(px(7.0))
        .bg(theme.primary)
        .text_color(theme.primary_foreground)
        .text_size(Rems(0.74))
        .font_semibold()
        .cursor_pointer()
        .hover(|this| this.bg(theme.primary_hover))
        .child(chrome_icon(IconName::Plus, 17.0, theme.primary_foreground))
        .child("New Project")
        .on_click(cx.listener(|this, event, window, cx| {
            this.go_new(event, window, cx);
        }))
        .into_any_element()
}

fn render_open_button(
    id: impl Into<gpui::ElementId>,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    h_flex()
        .id(id)
        .items_center()
        .gap(px(7.0))
        .h(px(32.0))
        .px(px(14.0))
        .rounded(px(7.0))
        .bg(theme.secondary)
        .border_1()
        .border_color(theme.border)
        .text_color(theme.foreground)
        .text_size(Rems(0.74))
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary_hover))
        .child(chrome_icon(
            IconName::FolderOpen,
            16.0,
            theme.muted_foreground,
        ))
        .child("Open...")
        .on_click(cx.listener(ProjectManagerView::open_existing_project_via_picker))
        .into_any_element()
}

fn render_empty_state(theme: &Theme) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .gap(px(14.0))
        .child(
            div()
                .flex()
                .items_center()
                .justify_center()
                .w(px(72.0))
                .h(px(72.0))
                .rounded(px(16.0))
                .bg(theme.sidebar)
                .border_1()
                .border_color(theme.border)
                .child(chrome_icon(
                    IconName::FolderOpen,
                    32.0,
                    theme.muted_foreground,
                )),
        )
        .child(
            v_flex()
                .items_center()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(Rems(0.92))
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child("No recent projects"),
                )
                .child(
                    div()
                        .text_size(Rems(0.74))
                        .text_color(theme.muted_foreground)
                        .child("Create a new project or open an existing one to get started."),
                ),
        )
        .into_any_element()
}

fn render_project_list(
    projects: Vec<RecentItem>,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let rows = projects.into_iter().fold(
        v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px(px(14.0))
            .py(px(6.0)),
        |rows, project| rows.child(render_project_row(&project, theme, cx)),
    );

    v_flex()
        .flex_1()
        .min_h_0()
        .child(
            h_flex()
                .flex_none()
                .items_center()
                .h(px(26.0))
                .px(px(24.0))
                .border_b_1()
                .border_color(theme.table_row_border)
                .text_size(Rems(0.64))
                .text_color(theme.table_head_foreground)
                .child(div().w(px(40.0)).flex_none())
                .child(div().flex_1().child("PROJECT"))
                .child(render_column_heading("ENGINE", 90.0))
                .child(render_column_heading("LAST OPENED", 104.0)),
        )
        .child(rows)
        .into_any_element()
}

fn render_project_row(
    project: &RecentItem,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let open = project.clone();

    h_flex()
        .id(format!("recent-project-{}", project.id))
        .items_center()
        .h(px(52.0))
        .px(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(gpui::transparent_black())
        .cursor_pointer()
        .hover(|this| this.bg(theme.sidebar).border_color(theme.border))
        .child(
            div()
                .w(px(40.0))
                .flex_none()
                .flex()
                .justify_center()
                .child(render_project_icon(project, theme)),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .min_w_0()
                                .text_size(Rems(0.78))
                                .font_medium()
                                .text_color(theme.foreground)
                                .overflow_hidden()
                                .text_ellipsis()
                                .child(project.name.clone()),
                        )
                        .when(project.pinned, |this| {
                            this.child(chrome_icon(IconName::PushPin, 13.0, theme.warning))
                        }),
                )
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(Rems(0.66))
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(project.path.clone()),
                ),
        )
        .child(
            div()
                .w(px(90.0))
                .flex_none()
                .child(render_engine_badge(&project.ver, theme)),
        )
        .child(render_text_column(&project.opened, 104.0, theme))
        .child(render_pin_button(project, theme, cx))
        .on_click(cx.listener(move |_this, event, window, cx| {
            open.on_click(event, window, cx);
        }))
        .into_any_element()
}

fn render_project_grid(
    projects: Vec<RecentItem>,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    projects
        .into_iter()
        .fold(
            h_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .flex_wrap()
                .items_start()
                .gap(px(10.0))
                .p(px(18.0)),
            |grid, project| grid.child(render_project_card(&project, theme, cx)),
        )
        .into_any_element()
}

fn render_project_card(
    project: &RecentItem,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let open = project.clone();

    v_flex()
        .id(format!("recent-project-card-{}", project.id))
        .w(px(236.0))
        .min_h(px(132.0))
        .gap(px(8.0))
        .p(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .cursor_pointer()
        .hover(|this| this.border_color(theme.accent))
        .child(
            h_flex()
                .items_center()
                .gap(px(9.0))
                .child(render_project_icon(project, theme))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(Rems(0.8))
                        .font_medium()
                        .text_color(theme.foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(project.name.clone()),
                )
                .child(render_pin_button(project, theme, cx)),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(Rems(0.66))
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .child(project.path.clone()),
        )
        .child(div().flex_1())
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(render_engine_badge(&project.ver, theme))
                .child(
                    div()
                        .text_size(Rems(0.68))
                        .text_color(theme.sidebar_foreground)
                        .child(project.opened.clone()),
                ),
        )
        .on_click(cx.listener(move |_this, event, window, cx| {
            open.on_click(event, window, cx);
        }))
        .into_any_element()
}

fn render_pin_button(
    project: &RecentItem,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let project_id = project.id.clone();
    let icon_color = if project.pinned {
        theme.warning
    } else {
        theme.muted_foreground
    };

    div()
        .id(format!("recent-project-pin-{}", project.id))
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.0))
        .h(px(28.0))
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary))
        .child(chrome_icon(IconName::PushPin, 15.0, icon_color))
        .tooltip(chrome_tooltip(if project.pinned {
            "Unpin project"
        } else {
            "Pin project"
        }))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_recent_project_pin(&project_id);
            cx.notify();
        }))
        .into_any_element()
}

fn render_sort_chip(
    active: ManagerRecentSort,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    h_flex()
        .id("manager-sort")
        .items_center()
        .gap(px(5.0))
        .h(px(26.0))
        .px(px(9.0))
        .rounded(px(6.0))
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(|this| this.bg(theme.sidebar).text_color(theme.sidebar_foreground))
        .child("Sort:")
        .child(
            div()
                .text_color(theme.sidebar_foreground)
                .child(active.label()),
        )
        .child(chrome_icon(
            IconName::ChevronsUpDown,
            14.0,
            theme.muted_foreground,
        ))
        .tooltip(chrome_tooltip("Cycle recent-project sort"))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_recent_sort(active.next());
            cx.notify();
        }))
        .into_any_element()
}

fn render_layout_toggle(
    active: ManagerRecentLayout,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    h_flex()
        .id("manager-recent-view-toggle")
        .flex_none()
        .items_center()
        .p(px(2.0))
        .rounded(px(6.0))
        .bg(theme.sidebar)
        .child(render_layout_button(
            "manager-recent-list-view",
            IconName::ViewList,
            "List view",
            active == ManagerRecentLayout::List,
            ManagerRecentLayout::List,
            theme,
            cx,
        ))
        .child(render_layout_button(
            "manager-recent-grid-view",
            IconName::GridView,
            "Grid view",
            active == ManagerRecentLayout::Grid,
            ManagerRecentLayout::Grid,
            theme,
            cx,
        ))
        .into_any_element()
}

fn render_layout_button(
    id: impl Into<gpui::ElementId>,
    icon: IconName,
    tooltip: &'static str,
    active: bool,
    target: ManagerRecentLayout,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let icon_color = if active {
        theme.primary_foreground
    } else {
        theme.muted_foreground
    };

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(26.0))
        .h(px(21.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .when(active, |this| this.bg(theme.primary_active))
        .when(!active, |this| {
            this.hover(|this| this.text_color(theme.foreground))
        })
        .child(chrome_icon(icon, 16.0, icon_color))
        .tooltip(chrome_tooltip(tooltip))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_recent_layout(target);
            cx.notify();
        }))
        .into_any_element()
}

fn render_column_heading(label: &'static str, width: f32) -> AnyElement {
    div()
        .w(px(width))
        .flex_none()
        .child(label)
        .into_any_element()
}

fn render_text_column(value: &str, width: f32, theme: &Theme) -> AnyElement {
    div()
        .w(px(width))
        .flex_none()
        .text_size(Rems(0.7))
        .text_color(theme.muted_foreground)
        .overflow_hidden()
        .text_ellipsis()
        .child(value.to_string())
        .into_any_element()
}

fn render_project_icon(project: &RecentItem, theme: &Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(30.0))
        .h(px(30.0))
        .rounded(px(7.0))
        .bg(theme.accent)
        .child(chrome_icon(
            project.icon.clone(),
            17.0,
            theme.primary_foreground,
        ))
        .into_any_element()
}

fn render_engine_badge(version: &str, theme: &Theme) -> AnyElement {
    let current = version == env!("CARGO_PKG_VERSION");
    div()
        .font_family(theme.mono_font_family.clone())
        .text_size(Rems(0.64))
        .px(px(7.0))
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(if current {
            theme.success.opacity(0.12)
        } else {
            theme.secondary
        })
        .text_color(if current {
            theme.success
        } else {
            theme.muted_foreground
        })
        .child(version.to_string())
        .into_any_element()
}
