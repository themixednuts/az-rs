use super::super::project_manager::{Gem, Opt, ProjectManagerView, Step, Template};
use super::{footer, summary};
use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, px,
};
use gpui_component::{
    Icon, StyledExt as _, h_flex, scroll::ScrollableElement as _, theme::Theme, v_flex,
};

pub(super) fn render_new_project_region(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let step = if view.step1() {
        render_template_step(view, theme, cx)
    } else if view.step2() {
        render_configuration_step(view, theme, cx)
    } else {
        render_gems_step(view, theme, cx)
    };

    h_flex()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(theme.background)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(render_new_project_header(view, theme, cx))
                .child(step)
                .child(footer::render_footer_region(view, theme, cx)),
        )
        .child(summary::render_summary_region(view, theme))
        .into_any_element()
}

fn render_new_project_header(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    v_flex()
        .flex_none()
        .gap(px(14.0))
        .px(px(24.0))
        .pt(px(20.0))
        .pb(px(16.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            h_flex()
                .items_center()
                .child(
                    v_flex()
                        .gap(px(3.0))
                        .child(
                            gpui::div()
                                .text_size(px(18.0))
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child("Create a project"),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(11.0))
                                .text_color(theme.muted_foreground)
                                .child(view.path_preview()),
                        ),
                )
                .child(gpui::div().flex_1())
                .child(
                    h_flex()
                        .id("project-manager-open-recent")
                        .items_center()
                        .h(px(30.0))
                        .px(px(10.0))
                        .rounded(px(6.0))
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.secondary).text_color(theme.foreground))
                        .on_click(cx.listener(|this, event, window, cx| {
                            this.go_recent(event, window, cx);
                        }))
                        .child("Recent projects"),
                ),
        )
        .child(
            h_flex().items_center().gap(px(6.0)).children(
                view.steps()
                    .into_iter()
                    .map(|step| render_step_item(step, theme, cx)),
            ),
        )
        .into_any_element()
}

fn render_step_item(step: Step, theme: &Theme, cx: &Context<'_, ProjectManagerView>) -> AnyElement {
    let color = if step.done {
        theme.primary
    } else if step.not_done {
        theme.muted_foreground
    } else {
        theme.foreground
    };
    let handler = step.clone();

    h_flex()
        .id(format!("project-manager-step-{}", step.num))
        .items_center()
        .gap(px(6.0))
        .h(px(28.0))
        .px(px(9.0))
        .rounded(px(6.0))
        .text_size(px(11.0))
        .text_color(color)
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary))
        .on_click(cx.listener(move |_this, event, window, cx| {
            handler.on_click(event, window, cx);
        }))
        .child(
            gpui::div()
                .w(px(17.0))
                .h(px(17.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(if step.done {
                    theme.primary
                } else {
                    theme.secondary
                })
                .text_color(if step.done {
                    theme.primary_foreground
                } else {
                    color
                })
                .text_size(px(9.0))
                .child(step.num.to_string()),
        )
        .child(step.label)
        .into_any_element()
}

fn render_template_step(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .p(px(24.0))
        .gap(px(12.0))
        .child(step_intro(
            "Choose a template",
            "Start from a project shape that fits your game.",
            theme,
        ))
        .children(
            view.templates()
                .into_iter()
                .map(|template| render_template_item(template, theme, cx)),
        )
        .into_any_element()
}

fn render_template_item(
    template: Template,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    let handler = template.clone();
    let border = if template.selected {
        theme.primary
    } else {
        theme.border
    };

    h_flex()
        .id(format!("project-manager-template-{}", template.name))
        .items_center()
        .gap(px(12.0))
        .p(px(14.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(border)
        .bg(if template.selected {
            theme.secondary
        } else {
            theme.background
        })
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary))
        .on_click(cx.listener(move |_this, event, window, cx| {
            handler.on_click(event, window, cx);
        }))
        .child(Icon::new(template.icon).size(px(24.0)))
        .child(
            v_flex()
                .gap(px(3.0))
                .child(
                    gpui::div()
                        .text_size(px(12.0))
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child(template.name),
                )
                .child(
                    gpui::div()
                        .text_size(px(10.5))
                        .text_color(theme.muted_foreground)
                        .child(template.desc),
                ),
        )
        .into_any_element()
}

fn render_configuration_step(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .p(px(24.0))
        .gap(px(18.0))
        .child(step_intro(
            "Configure the project",
            "Review the runtime choices before creating files.",
            theme,
        ))
        .child(render_options("Topology", view.topology_opts(), theme, cx))
        .child(render_options(
            "Renderer",
            ProjectManagerView::renderer_opts(),
            theme,
            cx,
        ))
        .child(render_options(
            "Render pipeline",
            ProjectManagerView::pipeline_opts(),
            theme,
            cx,
        ))
        .child(render_options(
            "Color space",
            ProjectManagerView::color_opts(),
            theme,
            cx,
        ))
        .child(render_options(
            "Target platforms",
            ProjectManagerView::platform_opts(),
            theme,
            cx,
        ))
        .into_any_element()
}

fn render_options(
    title: &str,
    options: Vec<Opt>,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    v_flex()
        .gap(px(8.0))
        .child(
            gpui::div()
                .text_size(px(11.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(title.to_string()),
        )
        .children(options.into_iter().map(|option| {
            let handler = option.clone();
            h_flex()
                .id(format!("project-manager-option-{}", option.label))
                .items_center()
                .gap(px(8.0))
                .h(px(32.0))
                .px(px(10.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(theme.border)
                .text_size(px(10.5))
                .text_color(theme.foreground)
                .cursor_pointer()
                .hover(|this| this.bg(theme.secondary))
                .on_click(cx.listener(move |_this, event, window, cx| {
                    handler.on_click(event, window, cx);
                }))
                .child(Icon::new(option.icon).size(px(15.0)))
                .child(option.label)
        }))
        .into_any_element()
}

fn render_gems_step(
    view: &ProjectManagerView,
    theme: &Theme,
    cx: &Context<'_, ProjectManagerView>,
) -> AnyElement {
    v_flex()
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .p(px(24.0))
        .gap(px(18.0))
        .child(step_intro(
            "Choose gems",
            "Required gems stay enabled. Select optional engine features here.",
            theme,
        ))
        .children(view.gem_groups().into_iter().map(|group| {
            let heading = format!("{} ({})", group.cat, group.count);
            v_flex()
                .gap(px(7.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(7.0))
                        .text_size(px(11.0))
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child(Icon::new(group.icon).size(px(15.0)))
                        .child(heading),
                )
                .children(
                    group
                        .items
                        .into_iter()
                        .map(|gem| render_gem_item(gem, theme, cx)),
                )
        }))
        .into_any_element()
}

fn render_gem_item(gem: Gem, theme: &Theme, cx: &Context<'_, ProjectManagerView>) -> AnyElement {
    let handler = gem.clone();
    let state = if gem.locked { "Required" } else { "Optional" };

    h_flex()
        .id(format!("project-manager-gem-{}", gem.name))
        .items_center()
        .gap(px(10.0))
        .p(px(10.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .text_size(px(10.5))
        .text_color(theme.foreground)
        .cursor_pointer()
        .hover(|this| this.bg(theme.secondary))
        .on_click(cx.listener(move |_this, event, window, cx| {
            handler.on_toggle(event, window, cx);
        }))
        .child(
            v_flex()
                .flex_1()
                .gap(px(2.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            gpui::div()
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child(gem.name),
                        )
                        .child(
                            gpui::div()
                                .text_size(px(9.5))
                                .text_color(theme.muted_foreground)
                                .child(gem.ver),
                        ),
                )
                .child(
                    gpui::div()
                        .text_size(px(9.5))
                        .text_color(theme.muted_foreground)
                        .child(gem.desc),
                ),
        )
        .child(
            gpui::div()
                .text_size(px(9.5))
                .text_color(if gem.locked {
                    theme.primary
                } else {
                    theme.muted_foreground
                })
                .child(state),
        )
        .into_any_element()
}

fn step_intro(title: &str, detail: &str, theme: &Theme) -> AnyElement {
    v_flex()
        .gap(px(3.0))
        .child(
            gpui::div()
                .text_size(px(14.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(title.to_string()),
        )
        .child(
            gpui::div()
                .text_size(px(10.5))
                .text_color(theme.muted_foreground)
                .child(detail.to_string()),
        )
        .into_any_element()
}
