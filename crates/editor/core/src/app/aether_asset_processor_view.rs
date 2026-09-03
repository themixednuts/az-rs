//! Adopted Aether Asset Processor view.
//!
//! The design source is `design/aether-handoff/project/Aether Asset Processor.dc.html`.
//! This file is owned code: data is projected from editor globals and actions.

use std::path::{Path, PathBuf};

use az_editor_ui::actions;
use az_editor_ui::panels::EditorAssetBrowserStatus;
use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::scroll::ScrollableElement as _;
use gpui_component::{
    ActiveTheme as _, Icon, Sizable as _, StyledExt as _, TitleBar, h_flex, v_flex,
};

use super::aether_asset_processor_model::{
    AetherAssetProcessorAction, AetherAssetProcessorBuilderProjection,
    AetherAssetProcessorDiagnosticFilter, AetherAssetProcessorHeader,
    AetherAssetProcessorProductProjection, AetherAssetProcessorState, AetherAssetProcessorTab,
};
use super::aether_common::AetherItem;
use crate::attach::EditorAttachSession;

pub struct AetherAssetProcessorView {
    pub(crate) state: AetherAssetProcessorState,
    job_filter_input: Entity<InputState>,
    source_filter_input: Entity<InputState>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl AetherAssetProcessorView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            state: AetherAssetProcessorState::new(),
            job_filter_input: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Filter by source or job key...")
            }),
            source_filter_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("Search assets...")),
            _subscriptions: Vec::new(),
        };
        let subscription = cx.subscribe_in(
            &this.job_filter_input,
            window,
            |this: &mut Self, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_action(AetherAssetProcessorAction::SetJobQuery(
                        input.read(cx).value().to_string(),
                    ));
                }
            },
        );
        this._subscriptions.push(subscription);
        let subscription = cx.subscribe_in(
            &this.source_filter_input,
            window,
            |this: &mut Self, input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_action(AetherAssetProcessorAction::SetSourceQuery(
                        input.read(cx).value().to_string(),
                    ));
                }
            },
        );
        this._subscriptions.push(subscription);
        this
    }

    fn window_title(&self, cx: &mut Context<Self>) -> String {
        let project = cx
            .try_global::<EditorAttachSession>()
            .map(attached_project_window_name)
            .unwrap_or_else(|| "project".to_owned());
        format!("Aether Asset Processor — {project}")
    }
}

impl Render for AetherAssetProcessorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.window_title(cx);
        window.set_window_title(&title);
        let theme = cx.theme().clone();
        let header = self.header(cx);

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .text_size(px(12.0))
            .child(self.render_titlebar(&theme))
            .child(self.render_status_strip(header, window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    .items_start()
                    .child(self.render_nav(cx))
                    .child(self.render_active_tab(window, cx)),
            )
            .child(self.render_bottom_bar(cx))
    }
}

impl AetherAssetProcessorView {
    fn render_titlebar(&self, theme: &gpui_component::theme::Theme) -> impl IntoElement {
        TitleBar::new().child(
            h_flex()
                .items_center()
                .child(
                    div()
                        .w(px(14.0))
                        .h(px(14.0))
                        .rounded(px(2.0))
                        .mr(px(11.0))
                        .bg(theme.accent),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child("Aether"),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(theme.muted_foreground)
                        .ml(px(7.0))
                        .child("Asset Processor"),
                ),
        )
    }

    fn render_status_strip(
        &mut self,
        header: AetherAssetProcessorHeader,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let counts = self.counts(cx);
        let fill = gpui::relative(f32::from(header.percent) / 100.0);

        h_flex()
            .flex_none()
            .items_center()
            .gap(px(18.0))
            .px(px(18.0))
            .py(px(11.0))
            .bg(theme.sidebar)
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(11.0))
                    .min_w_0()
                    .child(material_icon(header.icon).with_size(px(26.0)).text_color(
                        if header.busy {
                            theme.accent
                        } else if counts.failed > 0 || counts.warnings > 0 {
                            theme.warning
                        } else {
                            theme.success
                        },
                    ))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_semibold()
                                    .text_color(theme.foreground)
                                    .child(header.title),
                            )
                            .child(
                                div()
                                    .mt(px(1.0))
                                    .text_size(px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child(header.subtitle),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .max_w(px(360.0))
                    .min_w(px(140.0))
                    .gap(px(5.0))
                    .child(
                        div()
                            .h(px(6.0))
                            .rounded(px(4.0))
                            .bg(theme.muted)
                            .overflow_hidden()
                            .when(header.progress_known, |this| {
                                this.child(div().h_full().w(fill).rounded(px(4.0)).bg(theme.accent))
                            }),
                    )
                    .child(
                        h_flex()
                            .justify_between()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(px(10.0))
                            .text_color(theme.muted_foreground)
                            .child(header.completed_label)
                            .child(header.percent_label),
                    ),
            )
            .child(
                h_flex()
                    .ml_auto()
                    .items_center()
                    .gap(px(7.0))
                    .children([
                        stat_chip(
                            "check_circle",
                            counts.succeeded,
                            "Done",
                            theme.success,
                            &theme,
                        ),
                        stat_chip("sync", counts.active, "Active", theme.accent, &theme),
                        stat_chip(
                            "schedule",
                            counts.queued,
                            "Queued",
                            theme.muted_foreground,
                            &theme,
                        ),
                        stat_chip("warning", counts.warnings, "Warn", theme.warning, &theme),
                        stat_chip("error", counts.failed, "Failed", theme.danger, &theme),
                        stat_chip(
                            "construction",
                            counts.builders,
                            "Builders",
                            theme.muted_foreground,
                            &theme,
                        ),
                        stat_chip(
                            "lan",
                            counts.connections,
                            "Conn",
                            theme.muted_foreground,
                            &theme,
                        ),
                    ])
                    .child(refresh_button(cx)),
            )
    }

    fn render_nav(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let counts = self.counts(cx);
        let mut nav = v_flex()
            .w(px(188.0))
            .h_full()
            .flex_none()
            .min_h_0()
            .p(px(10.0))
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border);

        for tab in self.tabs(cx) {
            nav = nav.child(self.render_nav_tab(tab, cx));
        }

        nav.child(
            v_flex()
                .mt_auto()
                .gap(px(6.0))
                .border_t_1()
                .border_color(theme.border)
                .pt(px(10.0))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(
                            material_icon("bolt")
                                .with_size(px(16.0))
                                .text_color(theme.success),
                        )
                        .child("Builder pool")
                        .child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.foreground)
                                .child(counts.builders.to_string()),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(
                            material_icon("folder")
                                .with_size(px(16.0))
                                .text_color(theme.muted_foreground),
                        )
                        .child(format!("{} roots", counts.roots)),
                ),
        )
    }

    fn render_nav_tab(&mut self, tab: AetherItem, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let key = tab.key.clone();
        let selected = tab.selected;
        let target_tab = match key.as_str() {
            "assets" => AetherAssetProcessorTab::Assets,
            "logs" => AetherAssetProcessorTab::Logs,
            "builders" => AetherAssetProcessorTab::Builders,
            "connections" => AetherAssetProcessorTab::Connections,
            _ => AetherAssetProcessorTab::Jobs,
        };
        h_flex()
            .id(format!("asset-processor-tab-{key}"))
            .items_center()
            .gap(px(10.0))
            .h(px(38.0))
            .px(px(11.0))
            .mb(px(3.0))
            .rounded(px(8.0))
            .cursor_default()
            .text_color(if selected {
                theme.foreground
            } else {
                theme.muted_foreground
            })
            .when(selected, |this| this.bg(theme.accent.opacity(0.14)))
            .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, _window, cx| {
                    this.apply_action(AetherAssetProcessorAction::ShowTab(target_tab));
                    cx.stop_propagation();
                }),
            )
            .child(material_icon(tab.icon).with_size(px(18.0)))
            .child(div().flex_1().child(tab.label))
            .when(tab.has_badge, |this| {
                this.child(
                    div()
                        .min_w(px(18.0))
                        .px(px(7.0))
                        .py(px(1.0))
                        .rounded(px(9.0))
                        .bg(if selected { theme.accent } else { theme.muted })
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(10.0))
                        .font_semibold()
                        .text_color(if selected {
                            theme.accent_foreground
                        } else {
                            theme.muted_foreground
                        })
                        .child(tab.badge),
                )
            })
    }

    fn render_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active_tab = match self.state.tab {
            AetherAssetProcessorTab::Jobs => self.render_jobs(cx).into_any_element(),
            AetherAssetProcessorTab::Assets => self.render_sources(window, cx).into_any_element(),
            AetherAssetProcessorTab::Logs => self.render_diagnostics(cx).into_any_element(),
            AetherAssetProcessorTab::Builders => self.render_builders(cx).into_any_element(),
            AetherAssetProcessorTab::Connections => self.render_connections(cx).into_any_element(),
        };

        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .h_full()
            .child(active_tab)
    }

    fn render_jobs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows = self.job_rows(cx);
        let rows_empty = rows.is_empty();
        let job_children = rows
            .into_iter()
            .map(|row| Self::render_job_row(row, cx).into_any_element())
            .collect::<Vec<_>>();
        v_flex()
            .size_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(self.render_jobs_toolbar(cx))
            .child(job_table_header(&theme))
            .child(
                v_flex()
                    .flex_1()
                    .min_h(px(150.0))
                    .overflow_y_scrollbar()
                    .when(rows_empty, |this| {
                        this.child(empty_state(
                            "filter_alt",
                            "No jobs match this filter",
                            &theme,
                        ))
                    })
                    .children(job_children),
            )
            .child(self.render_selected_job_events(cx))
    }

    fn render_jobs_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let selected_job = self.selected_job_summary(cx);
        h_flex()
            .flex_none()
            .items_center()
            .gap(px(9.0))
            .px(px(16.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .child(search_box(&self.job_filter_input, "search", &theme))
            .child(
                h_flex()
                    .gap(px(4.0))
                    .children(self.status_chips(cx).into_iter().map(|chip| {
                        let key = chip.key.clone();
                        let filter = match key.as_str() {
                            "active" => super::aether_asset_processor_model::AetherAssetProcessorJobFilter::Active,
                            "queued" => super::aether_asset_processor_model::AetherAssetProcessorJobFilter::Queued,
                            "done" => super::aether_asset_processor_model::AetherAssetProcessorJobFilter::Succeeded,
                            "warnings" => super::aether_asset_processor_model::AetherAssetProcessorJobFilter::Warnings,
                            "failed" => super::aether_asset_processor_model::AetherAssetProcessorJobFilter::Failed,
                            _ => super::aether_asset_processor_model::AetherAssetProcessorJobFilter::All,
                        };
                        filter_chip(chip.label, chip.count, chip.active, &theme).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _event, _window, cx| {
                                this.apply_action(AetherAssetProcessorAction::SetJobFilter(filter));
                                cx.stop_propagation();
                            }),
                        )
                    })),
            )
            .child(
                h_flex()
                    .ml_auto()
                    .items_center()
                    .gap(px(5.0))
                    .children(self.platform_options(cx).into_iter().map(|option| {
                        let platform = option.key.clone();
                        let active = self.state.platform_filter == option.key;
                        filter_chip(option.label, String::new(), active, &theme).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _event, window, cx| {
                                this.apply_action(AetherAssetProcessorAction::SetPlatformFilter(
                                    platform.clone(),
                                ));
                                window.dispatch_action(
                                    Box::new(actions::RefreshCatalogProducts {
                                        platform: platform.clone(),
                                    }),
                                    cx,
                                );
                                cx.stop_propagation();
                            }),
                        )
                    })),
            )
            .child(reprocess_button(selected_job.as_ref(), cx))
    }

    fn render_job_row(row: AetherItem, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let job_id = row.key.parse::<i64>().ok();
        let attempt_id = (!row.idx.is_empty())
            .then(|| row.idx.parse::<i64>().ok())
            .flatten();
        let selected = row.selected;
        h_flex()
            .id(format!("asset-processor-job-{}", row.key))
            .items_center()
            .h(px(40.0))
            .px(px(16.0))
            .border_b_1()
            .border_color(theme.border.opacity(0.55))
            .cursor_default()
            .when(selected, |this| this.bg(theme.accent.opacity(0.12)))
            .hover(|this| this.bg(theme.list_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, window, cx| {
                    if let Some(job_id) = job_id {
                        this.apply_action(AetherAssetProcessorAction::SelectJob(job_id));
                        window.dispatch_action(
                            Box::new(actions::InspectJob { job_id, attempt_id }),
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                h_flex()
                    .w(px(132.0))
                    .flex_none()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        material_icon(&row.icon)
                            .with_size(px(16.0))
                            .text_color(status_color(&row.kind, &theme)),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(status_color(&row.kind, &theme))
                            .child(row.status),
                    ),
            )
            .child(platform_badge(row.tag, &theme).w(px(92.0)).flex_none())
            .child(
                div()
                    .w(px(150.0))
                    .flex_none()
                    .text_size(px(11.5))
                    .text_color(theme.muted_foreground)
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(row.label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(12.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(row.src),
            )
            .child(
                div()
                    .w(px(130.0))
                    .flex_none()
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child(row.time),
            )
            .child(
                div()
                    .w(px(78.0))
                    .flex_none()
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child(row.unit),
            )
    }

    fn render_selected_job_events(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let events = self.selected_job_events(cx);
        let summary = self.selected_job_summary(cx);
        let source_key = summary.as_ref().map(|summary| summary.value.clone());
        let source_root = summary
            .as_ref()
            .map(|summary| summary.from.clone())
            .filter(|root| !root.trim().is_empty());
        let source_path = summary
            .as_ref()
            .map(|summary| summary.src.clone())
            .filter(|path| !path.trim().is_empty());
        let source_root_path = summary
            .as_ref()
            .map(|summary| summary.to.clone())
            .filter(|root| !root.trim().is_empty());
        let source_file_path =
            source_file_path(source_root_path.as_deref(), source_path.as_deref());
        let job_id = summary
            .as_ref()
            .and_then(|summary| summary.key.parse::<i64>().ok());
        let attempt_id = summary
            .as_ref()
            .filter(|summary| !summary.idx.is_empty())
            .and_then(|summary| summary.idx.parse::<i64>().ok());
        v_flex()
            .flex_none()
            .h(px(220.0))
            .min_h(px(120.0))
            .border_t_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(16.0))
                    .py(px(9.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .child("Event Log"),
                    )
                    .when_some(summary, |this, summary| {
                        this.child(
                            div()
                                .min_w_0()
                                .max_w(px(360.0))
                                .font_family(theme.mono_font_family.clone())
                                .text_size(px(11.0))
                                .text_color(theme.foreground)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(summary.src),
                        )
                        .child(platform_badge(summary.tag, &theme))
                    })
                    .child(
                        h_flex()
                            .ml_auto()
                            .items_center()
                            .gap(px(6.0))
                            .child(event_reprocess_button(
                                source_root.clone(),
                                source_path.clone(),
                                cx,
                            ))
                            .when_some(source_key, |this, source_key| {
                                this.child(event_view_source_button(
                                    source_key, job_id, attempt_id, cx,
                                ))
                            })
                            .child(event_reveal_button(source_file_path.clone(), &theme, cx))
                            .child(event_copy_path_button(source_file_path, &theme, cx))
                            .child(
                                div()
                                    .ml(px(6.0))
                                    .text_size(px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child(if events.is_empty() {
                                        "No asset-processor events for selection".to_owned()
                                    } else {
                                        format!("{} events", events.len())
                                    }),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(events.is_empty(), |this| {
                        this.child(empty_state(
                            "description",
                            "Select a job after live events arrive",
                            &theme,
                        ))
                    })
                    .children(
                        events
                            .into_iter()
                            .map(|event| diagnostic_line(event, &theme)),
                    ),
            )
    }

    fn render_sources(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let groups = self.source_groups(cx);
        let groups_empty = groups.is_empty();
        let mut source_children = Vec::new();
        for group in groups {
            source_children.push(source_group_header(group.label, &theme).into_any_element());
            for item in group.items {
                source_children.push(Self::render_source_row(item, cx).into_any_element());
            }
        }
        h_flex()
            .size_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .items_start()
            .bg(theme.background)
            .child(
                v_flex()
                    .w(px(326.0))
                    .h_full()
                    .flex_none()
                    .min_h_0()
                    .bg(theme.sidebar)
                    .border_r_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_none()
                            .p(px(12.0))
                            .border_b_1()
                            .border_color(theme.border)
                            .child(search_box(&self.source_filter_input, "search", &theme)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .when(groups_empty, |this| {
                                this.child(empty_state(
                                    "inventory_2",
                                    "No assets match this filter",
                                    &theme,
                                ))
                            })
                            .children(source_children),
                    ),
            )
            .child(self.render_source_detail(cx))
            .child(self.render_catalog_products(cx))
    }

    fn render_source_row(row: AetherItem, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let source_key = row.key.clone();
        let job = if row.tag.is_empty() {
            None
        } else {
            latest_job_for_source(cx, &source_key)
        };
        h_flex()
            .id(format!("asset-processor-source-{}", row.key))
            .items_center()
            .gap(px(9.0))
            .h(px(32.0))
            .px(px(14.0))
            .cursor_default()
            .when(row.selected, |this| this.bg(theme.accent.opacity(0.12)))
            .hover(|this| this.bg(theme.list_hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _event, window, cx| {
                    this.apply_action(AetherAssetProcessorAction::SelectSource {
                        source_key: source_key.clone(),
                        job_id: job.map(|(job_id, _)| job_id),
                    });
                    if let Some((job_id, attempt_id)) = job {
                        window.dispatch_action(
                            Box::new(actions::InspectJob { job_id, attempt_id }),
                            cx,
                        );
                    }
                    cx.stop_propagation();
                }),
            )
            .child(
                material_icon(&row.icon)
                    .with_size(px(16.0))
                    .text_color(status_color(&row.kind, &theme)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(12.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(row.name),
            )
            .child(
                material_icon(status_icon_for_kind(&row.kind))
                    .with_size(px(14.0))
                    .text_color(status_color(&row.kind, &theme)),
            )
    }

    fn render_source_detail(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let Some(detail) = self.selected_source_detail(cx) else {
            return div()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .h_full()
                .child(empty_state("inventory_2", "No source selected", &theme))
                .into_any_element();
        };
        let source_root =
            (!detail.source_root.trim().is_empty()).then(|| detail.source_root.clone());
        let source_path =
            (!detail.source_path.trim().is_empty()).then(|| detail.source_path.clone());
        let source_file_path = source_file_path(
            (!detail.source_root_path.trim().is_empty())
                .then_some(detail.source_root_path.as_str()),
            source_path.as_deref(),
        );
        v_flex()
            .h_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .overflow_y_scrollbar()
            .p(px(24.0))
            .gap(px(20.0))
            .child(
                h_flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .size(px(44.0))
                            .flex_none()
                            .rounded(px(8.0))
                            .bg(theme.muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                material_icon(detail.icon)
                                    .with_size(px(24.0))
                                    .text_color(theme.accent),
                            ),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_size(px(16.0))
                                    .font_semibold()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(detail.title),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .font_family(theme.mono_font_family.clone())
                                    .text_size(px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child(detail.folder),
                            ),
                    )
                    .child(
                        h_flex()
                            .ml_auto()
                            .items_center()
                            .gap(px(6.0))
                            .child(event_reprocess_button(source_root, source_path, cx))
                            .child(event_reveal_button(source_file_path.clone(), &theme, cx))
                            .child(event_copy_path_button(source_file_path, &theme, cx))
                            .child(
                                h_flex()
                                    .items_center()
                                    .gap(px(6.0))
                                    .px(px(10.0))
                                    .py(px(4.0))
                                    .rounded(px(7.0))
                                    .bg(theme.muted)
                                    .text_color(theme.foreground)
                                    .child(material_icon(detail.status_icon).with_size(px(14.0)))
                                    .child(detail.status_label),
                            ),
                    ),
            )
            .child(meta_table(detail.meta, &theme))
            .child(section_list(
                "Product Assets",
                detail.products,
                detail.products_pending_inspection,
                "No inspected products for the selected source",
                &theme,
            ))
            .child(section_list(
                "Source Dependencies",
                detail.dependencies,
                false,
                "No source dependencies recorded",
                &theme,
            ))
            .into_any_element()
    }

    fn render_catalog_products(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let AetherAssetProcessorProductProjection {
            platform,
            products,
            error,
        } = self.catalog_products(cx);
        let product_count = products.len();
        v_flex()
            .w(px(390.0))
            .h_full()
            .flex_none()
            .min_w(px(300.0))
            .min_h_0()
            .bg(theme.sidebar)
            .border_l_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(14.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_semibold()
                                    .text_color(theme.foreground)
                                    .child("Product Assets"),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .font_family(theme.mono_font_family.clone())
                                    .text_size(px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{product_count} cataloged · {platform}")),
                            ),
                    )
                    .child(
                        h_flex()
                            .ml_auto()
                            .items_center()
                            .gap(px(6.0))
                            .child(catalog_products_refresh_button(platform, &theme, cx)),
                    ),
            )
            .children(error.map(|error| asset_processor_error_strip(error, &theme)))
            .child(product_table_header(&theme))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(products.is_empty(), |this| {
                        this.child(empty_state(
                            "deployed_code",
                            "No catalog products for this platform",
                            &theme,
                        ))
                    })
                    .children(
                        products
                            .into_iter()
                            .map(|product| product_asset_row(product, &theme)),
                    ),
            )
    }

    fn render_diagnostics(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows = self.diagnostic_rows(cx);
        v_flex()
            .size_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(16.0))
                    .py(px(10.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .children(self.diagnostic_tabs(cx).into_iter().map(|tab| {
                        let key = tab.key.clone();
                        let filter = match key.as_str() {
                            "messages" => AetherAssetProcessorDiagnosticFilter::Messages,
                            "warnings" => AetherAssetProcessorDiagnosticFilter::Warnings,
                            "errors" => AetherAssetProcessorDiagnosticFilter::Errors,
                            _ => AetherAssetProcessorDiagnosticFilter::All,
                        };
                        filter_chip(tab.label, tab.count, tab.active, &theme).on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _event, _window, cx| {
                                this.apply_action(AetherAssetProcessorAction::SetDiagnosticFilter(
                                    filter,
                                ));
                                cx.stop_propagation();
                            }),
                        )
                    }))
                    .child(
                        h_flex()
                            .ml_auto()
                            .items_center()
                            .gap(px(6.0))
                            .text_size(px(11.0))
                            .text_color(theme.muted_foreground)
                            .child(
                                material_icon("lens")
                                    .with_size(px(12.0))
                                    .text_color(theme.success),
                            )
                            .child("Live tail"),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(rows.is_empty(), |this| {
                        this.child(empty_state("description", "No asset logs recorded", &theme))
                    })
                    .children(rows.into_iter().map(|row| diagnostic_line(row, &theme))),
            )
    }

    fn render_connections(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let AetherAssetProcessorBuilderProjection {
            processes,
            allow_list,
            reject_list,
            ..
        } = self.builders(cx);
        let running = processes
            .iter()
            .filter(|process| process.kind == "active")
            .count();
        v_flex()
            .size_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(18.0))
                    .py(px(13.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_semibold()
                                    .text_color(theme.foreground)
                                    .child("Connections"),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{} running · {} current endpoints",
                                        running,
                                        processes.len()
                                    )),
                            ),
                    )
                    .child(
                        h_flex()
                            .ml_auto()
                            .items_center()
                            .gap(px(6.0))
                            .child(connection_refresh_button(&theme, cx))
                            .child(connection_start_button(&theme, cx))
                            .child(connection_stop_button(&theme, cx)),
                    ),
            )
            .child(connection_table_header(&theme))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .when(processes.is_empty(), |this| {
                        this.child(empty_state(
                            "lan",
                            "No asset-processor connections reported",
                            &theme,
                        ))
                    })
                    .children(
                        processes
                            .into_iter()
                            .map(|item| connection_row(item, &theme)),
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .items_start()
                    .gap(px(12.0))
                    .p(px(14.0))
                    .border_t_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .child(policy_list("Allowed", "verified", allow_list, &theme))
                    .child(policy_list("Rejected", "block", reject_list, &theme)),
            )
    }

    fn render_builders(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let AetherAssetProcessorBuilderProjection {
            builders,
            schemas,
            processes,
            ..
        } = self.builders(cx);
        let builder_count = builders.len();
        let schema_count = schemas.len();
        let process_count = processes.len();
        v_flex()
            .size_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(
                h_flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.0))
                    .px(px(18.0))
                    .py(px(13.0))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_semibold()
                                    .text_color(theme.foreground)
                                    .child("Builders"),
                            )
                            .child(
                                div()
                                    .mt(px(2.0))
                                    .text_size(px(11.0))
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{builder_count} registered builders · {schema_count} source schemas · {process_count} current endpoints"
                                    )),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap(px(12.0))
                    .p(px(14.0))
                    .overflow_y_scrollbar()
                    .child(
                        h_flex()
                            .flex_none()
                            .gap(px(10.0))
                            .child(builder_metric_tile(
                                "Registered Builders",
                                "build",
                                builder_count,
                                "builder catalog entries",
                                &theme,
                            ))
                            .child(builder_metric_tile(
                                "Source Schemas",
                                "schema",
                                schema_count,
                                "editable source types",
                                &theme,
                            ))
                            .child(builder_metric_tile(
                                "Processes",
                                "lan",
                                process_count,
                                "current session endpoints",
                                &theme,
                            )),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .gap(px(12.0))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(12.0))
                                    .child(builder_section(
                                        "Registered Asset Builders",
                                        "build",
                                        "Schemas",
                                        builders,
                                        "No registered builders reported by the asset processor",
                                        &theme,
                                    ))
                                    .child(builder_section(
                                        "Source Schemas",
                                        "schema",
                                        "Templates",
                                        schemas,
                                        "No source schemas reported by the asset processor",
                                        &theme,
                                    )),
                            )
                            .child(
                                v_flex()
                                    .w(px(380.0))
                                    .flex_none()
                                    .min_w(px(300.0))
                                    .child(builder_process_section(processes, &theme)),
                            ),
                    ),
            )
    }

    fn render_bottom_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let counts = self.counts(cx);
        let watch_label = cx
            .try_global::<EditorAssetBrowserStatus>()
            .and_then(|status| status.roots.first())
            .map(|root| root.source_root.clone())
            .unwrap_or_else(|| "no source root loaded".to_owned());
        h_flex()
            .flex_none()
            .h(px(24.0))
            .items_center()
            .gap(px(16.0))
            .px(px(14.0))
            .bg(theme.tab_bar)
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(11.0))
            .text_color(theme.muted_foreground)
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(div().size(px(7.0)).rounded(px(4.0)).bg(
                        if counts.active + counts.queued > 0 {
                            theme.accent
                        } else {
                            theme.success
                        },
                    ))
                    .child(if counts.active + counts.queued > 0 {
                        "Asset jobs · running"
                    } else {
                        "Asset jobs · idle"
                    }),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(material_icon("visibility").with_size(px(14.0)))
                    .child("Watching")
                    .child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.foreground)
                            .child(watch_label),
                    ),
            )
            .child(
                div()
                    .ml_auto()
                    .font_family(theme.mono_font_family.clone())
                    .child(format!(
                        "{} jobs · {} done · {} queued · {} failed",
                        counts.jobs, counts.succeeded, counts.queued, counts.failed
                    )),
            )
    }
}

fn attached_project_window_name(session: &EditorAttachSession) -> String {
    session
        .project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&session.project_id)
        .to_owned()
}

fn material_icon(name: impl AsRef<str>) -> Icon {
    super::dc_icons::material_symbol_icon(name)
}

fn refresh_button(cx: &mut Context<AetherAssetProcessorView>) -> impl IntoElement {
    let theme = cx.theme().clone();
    h_flex()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(13.0))
        .ml(px(6.0))
        .rounded(px(7.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .text_size(px(12.0))
        .font_medium()
        .cursor_default()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|_this, _event, window, cx| {
                window.dispatch_action(Box::new(actions::ScanAssets), cx);
                cx.stop_propagation();
            }),
        )
        .child(material_icon("sync").with_size(px(16.0)))
        .child("Scan")
}

fn reprocess_button(
    summary: Option<&AetherItem>,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let source_root = summary
        .map(|summary| summary.from.clone())
        .filter(|root| !root.trim().is_empty());
    let source_path = summary
        .map(|summary| summary.src.clone())
        .filter(|path| !path.trim().is_empty());
    let can_reprocess = source_root.is_some() && source_path.is_some();
    h_flex()
        .items_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px(px(12.0))
        .rounded(px(7.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .text_size(px(12.0))
        .font_medium()
        .cursor_default()
        .when(!can_reprocess, |this| this.opacity(0.45))
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .when(can_reprocess, |this| {
            let source_root = source_root.clone().unwrap_or_default();
            let source_path = source_path.clone().unwrap_or_default();
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, window, cx| {
                    window.dispatch_action(
                        Box::new(actions::ForceReprocessAsset {
                            source_root: source_root.clone(),
                            source_path: source_path.clone(),
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
        })
        .child(material_icon("restart_alt").with_size(px(16.0)))
        .child("Reprocess")
}

fn event_reprocess_button(
    source_root: Option<String>,
    source_path: Option<String>,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    let can_reprocess = source_root.is_some() && source_path.is_some();
    event_action_button("Reprocess", "restart_alt", &theme)
        .when(!can_reprocess, |this| this.opacity(0.45))
        .when(can_reprocess, |this| {
            let source_root = source_root.clone().unwrap_or_default();
            let source_path = source_path.clone().unwrap_or_default();
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, window, cx| {
                    window.dispatch_action(
                        Box::new(actions::ForceReprocessAsset {
                            source_root: source_root.clone(),
                            source_path: source_path.clone(),
                        }),
                        cx,
                    );
                    cx.stop_propagation();
                }),
            )
        })
}

fn event_view_source_button(
    source_key: String,
    job_id: Option<i64>,
    attempt_id: Option<i64>,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    let theme = cx.theme().clone();
    event_action_button("View Source", "visibility", &theme).on_mouse_down(
        MouseButton::Left,
        cx.listener(move |this, _event, window, cx| {
            this.apply_action(AetherAssetProcessorAction::SelectSource {
                source_key: source_key.clone(),
                job_id,
            });
            if let Some(job_id) = job_id {
                window.dispatch_action(Box::new(actions::InspectJob { job_id, attempt_id }), cx);
            }
            cx.stop_propagation();
        }),
    )
}

fn event_reveal_button(
    path: Option<PathBuf>,
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherAssetProcessorView>,
) -> gpui::Div {
    let can_reveal = path.is_some();
    event_action_button("Open Location", "folder_open", theme)
        .when(!can_reveal, |this| this.opacity(0.45))
        .when_some(path, |this, path| {
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, _window, cx| {
                    cx.reveal_path(&path);
                    cx.stop_propagation();
                }),
            )
        })
}

fn event_copy_path_button(
    path: Option<PathBuf>,
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherAssetProcessorView>,
) -> gpui::Div {
    let can_copy = path.is_some();
    event_action_button("Copy Path", "content_copy", theme)
        .when(!can_copy, |this| this.opacity(0.45))
        .when_some(path, |this, path| {
            let path = path.display().to_string();
            this.on_mouse_down(
                MouseButton::Left,
                cx.listener(move |_this, _event, _window, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(path.clone()));
                    cx.stop_propagation();
                }),
            )
        })
}

fn source_file_path(source_root_path: Option<&str>, source_path: Option<&str>) -> Option<PathBuf> {
    let source_root_path = source_root_path?.trim();
    let source_path = source_path?.trim();
    if source_root_path.is_empty() || source_path.is_empty() {
        return None;
    }
    let source_path = Path::new(source_path);
    if source_path.is_absolute() {
        return None;
    }
    Some(PathBuf::from(source_root_path).join(source_path))
}

fn event_action_button(
    label: &'static str,
    icon: &'static str,
    theme: &gpui_component::theme::Theme,
) -> gpui::Div {
    h_flex()
        .items_center()
        .gap(px(5.0))
        .h(px(26.0))
        .px(px(9.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .text_size(px(11.0))
        .text_color(theme.muted_foreground)
        .cursor_default()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .child(material_icon(icon).with_size(px(14.0)))
        .child(label)
}

fn catalog_products_refresh_button(
    platform: String,
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    event_action_button("Refresh", "sync", theme).on_mouse_down(
        MouseButton::Left,
        cx.listener(move |_this, _event, window, cx| {
            window.dispatch_action(
                Box::new(actions::RefreshCatalogProducts {
                    platform: platform.clone(),
                }),
                cx,
            );
            cx.stop_propagation();
        }),
    )
}

fn asset_processor_error_strip(
    error: String,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .py(px(8.0))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.danger.opacity(0.08))
        .text_size(px(11.0))
        .text_color(theme.danger)
        .child(material_icon("error").with_size(px(14.0)))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(error),
        )
}

fn product_table_header(theme: &gpui_component::theme::Theme) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .h(px(28.0))
        .px(px(12.0))
        .border_b_1()
        .border_color(theme.border)
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(div().flex_1().min_w_0().child("Product"))
        .child(div().w(px(74.0)).flex_none().child("Platform"))
        .child(div().w(px(62.0)).flex_none().text_right().child("Size"))
}

fn product_asset_row(item: AetherItem, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    v_flex()
        .px(px(12.0))
        .py(px(9.0))
        .gap(px(4.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.5))
        .hover(|this| this.bg(theme.list_hover))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.0))
                .child(
                    material_icon(&item.icon)
                        .with_size(px(16.0))
                        .text_color(theme.accent),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.0))
                        .text_color(theme.foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(item.name),
                )
                .child(platform_badge(item.tag, theme).w(px(74.0)).flex_none())
                .child(
                    div()
                        .w(px(62.0))
                        .flex_none()
                        .text_right()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(item.count),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap(px(8.0))
                .pl(px(24.0))
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_font_family.clone())
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(item.src),
                )
                .child(product_meta_chip(item.label, theme))
                .when(!item.unit.is_empty(), |this| {
                    this.child(product_meta_chip(item.unit, theme))
                })
                .child(product_meta_chip(format!("sub {}", item.idx), theme)),
        )
        .child(
            div()
                .pl(px(24.0))
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.0))
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(format!("{} · {}", item.status, item.value)),
        )
}

fn product_meta_chip(
    label: impl Into<String>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(3.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .font_family(theme.mono_font_family.clone())
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(label.into())
}

fn connection_refresh_button(
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    connection_action_button("Refresh", "sync", theme).on_mouse_down(
        MouseButton::Left,
        cx.listener(|_this, _event, window, cx| {
            window.dispatch_action(Box::new(actions::RefreshSessionStatus), cx);
            cx.stop_propagation();
        }),
    )
}

fn connection_start_button(
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    connection_action_button("Start", "play_arrow", theme).on_mouse_down(
        MouseButton::Left,
        cx.listener(|_this, _event, window, cx| {
            window.dispatch_action(Box::new(actions::StartSessionServices), cx);
            cx.stop_propagation();
        }),
    )
}

fn connection_stop_button(
    theme: &gpui_component::theme::Theme,
    cx: &mut Context<AetherAssetProcessorView>,
) -> impl IntoElement {
    connection_action_button("Stop", "stop", theme).on_mouse_down(
        MouseButton::Left,
        cx.listener(|_this, _event, window, cx| {
            window.dispatch_action(Box::new(actions::StopSessionServices), cx);
            cx.stop_propagation();
        }),
    )
}

fn connection_action_button(
    label: &'static str,
    icon: &'static str,
    theme: &gpui_component::theme::Theme,
) -> gpui::Div {
    h_flex()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(12.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .text_color(theme.muted_foreground)
        .text_size(px(11.5))
        .font_medium()
        .cursor_default()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .child(material_icon(icon).with_size(px(15.0)))
        .child(label)
}

fn stat_chip(
    icon: &'static str,
    value: usize,
    label: &'static str,
    color: gpui::Hsla,
    theme: &gpui_component::theme::Theme,
) -> gpui::Div {
    h_flex()
        .items_center()
        .gap(px(6.0))
        .h(px(30.0))
        .px(px(11.0))
        .rounded(px(7.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .opacity(if value > 0 { 1.0 } else { 0.55 })
        .child(material_icon(icon).with_size(px(15.0)).text_color(color))
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.0))
                .font_medium()
                .text_color(if value > 0 {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .child(value.to_string()),
        )
        .child(
            div()
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(label),
        )
}

fn search_box(
    input: &Entity<InputState>,
    icon: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .w(px(240.0))
        .h(px(32.0))
        .items_center()
        .gap(px(7.0))
        .px(px(11.0))
        .rounded(px(7.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .child(
            material_icon(icon)
                .with_size(px(16.0))
                .text_color(theme.muted_foreground),
        )
        .child(
            div().flex_1().min_w_0().child(
                Input::new(input)
                    .small()
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
            ),
        )
}

fn filter_chip(
    label: String,
    count: String,
    active: bool,
    theme: &gpui_component::theme::Theme,
) -> gpui::Div {
    h_flex()
        .items_center()
        .gap(px(6.0))
        .h(px(32.0))
        .px(px(11.0))
        .rounded(px(7.0))
        .border_1()
        .border_color(if active { theme.accent } else { theme.border })
        .bg(if active {
            theme.accent.opacity(0.12)
        } else {
            theme.background
        })
        .text_color(if active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .cursor_default()
        .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
        .child(label)
        .when(!count.is_empty(), |this| {
            this.child(
                div()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(10.0))
                    .text_color(theme.muted_foreground)
                    .child(count),
            )
        })
}

fn job_table_header(theme: &gpui_component::theme::Theme) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .h(px(28.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme.border)
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(div().w(px(132.0)).flex_none().child("Status"))
        .child(div().w(px(92.0)).flex_none().child("Platform"))
        .child(div().w(px(150.0)).flex_none().child("Job Key"))
        .child(div().flex_1().child("Source Asset"))
        .child(div().w(px(130.0)).flex_none().child("Completed"))
        .child(div().w(px(78.0)).flex_none().text_right().child("Duration"))
}

fn platform_badge(label: String, theme: &gpui_component::theme::Theme) -> gpui::Div {
    div()
        .px(px(8.0))
        .py(px(2.0))
        .rounded(px(4.0))
        .bg(theme.accent.opacity(0.13))
        .font_family(theme.mono_font_family.clone())
        .text_size(px(10.0))
        .font_medium()
        .text_color(theme.accent)
        .child(label)
}

fn source_group_header(label: String, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(6.0))
        .px(px(14.0))
        .pt(px(9.0))
        .pb(px(4.0))
        .text_size(px(10.0))
        .font_semibold()
        .text_color(theme.muted_foreground)
        .child(material_icon("folder").with_size(px(14.0)))
        .child(label)
}

fn meta_table(items: Vec<AetherItem>, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    v_flex()
        .border_1()
        .border_color(theme.border)
        .rounded(px(8.0))
        .overflow_hidden()
        .children(items.into_iter().map(|item| {
            h_flex()
                .items_center()
                .gap(px(12.0))
                .px(px(14.0))
                .py(px(9.0))
                .bg(theme.sidebar)
                .border_b_1()
                .border_color(theme.border.opacity(0.5))
                .child(
                    div()
                        .w(px(120.0))
                        .flex_none()
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(item.label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(item.value),
                )
        }))
}

fn section_list(
    title: &'static str,
    items: Vec<AetherItem>,
    pending: bool,
    empty: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .gap(px(8.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(8.0))
                .text_size(px(11.0))
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(title)
                .child(
                    div()
                        .font_family(theme.mono_font_family.clone())
                        .child(items.len().to_string()),
                ),
        )
        .when(items.is_empty(), |this| {
            this.child(
                h_flex()
                    .items_center()
                    .gap(px(9.0))
                    .p(px(11.0))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .text_color(theme.muted_foreground)
                    .child(material_icon(if pending { "sync" } else { "info" }).with_size(px(16.0)))
                    .child(if pending {
                        "Inspecting selected job products..."
                    } else {
                        empty
                    }),
            )
        })
        .children(items.into_iter().map(|item| {
            h_flex()
                .items_center()
                .gap(px(10.0))
                .p(px(10.0))
                .rounded(px(8.0))
                .border_1()
                .border_color(theme.border)
                .bg(theme.sidebar)
                .child(
                    material_icon(item.icon)
                        .with_size(px(16.0))
                        .text_color(theme.accent),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_family(theme.mono_font_family.clone())
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(item.name),
                )
                .when(!item.tag.is_empty(), |this| {
                    this.child(platform_badge(item.tag, theme))
                })
                .when(!item.size.is_empty(), |this| {
                    this.child(
                        div()
                            .w(px(72.0))
                            .text_right()
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.muted_foreground)
                            .child(item.size),
                    )
                })
        }))
}

fn diagnostic_line(item: AetherItem, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    h_flex()
        .items_start()
        .gap(px(12.0))
        .px(px(16.0))
        .py(px(4.0))
        .hover(|this| this.bg(theme.list_hover))
        .child(
            div()
                .w(px(88.0))
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(item.time),
        )
        .child(
            div()
                .min_w(px(42.0))
                .px(px(6.0))
                .py(px(1.0))
                .rounded(px(4.0))
                .bg(status_color(&item.kind, theme).opacity(0.14))
                .text_size(px(9.5))
                .font_semibold()
                .text_color(status_color(&item.kind, theme))
                .child(item.tag),
        )
        .child(
            div()
                .w(px(128.0))
                .flex_none()
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(item.src),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.5))
                .text_color(if item.kind == "error" {
                    theme.danger
                } else if item.kind == "warning" {
                    theme.warning
                } else {
                    theme.foreground
                })
                .child(if item.msg.is_empty() {
                    item.name
                } else {
                    item.msg
                }),
        )
}

fn connection_table_header(theme: &gpui_component::theme::Theme) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .h(px(28.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme.border)
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(div().flex_1().min_w_0().child("Name"))
        .child(div().w(px(110.0)).flex_none().child("Type"))
        .child(div().w(px(92.0)).flex_none().child("Platform"))
        .child(div().w(px(170.0)).flex_none().child("Address"))
        .child(div().w(px(112.0)).flex_none().child("Status"))
        .child(div().w(px(64.0)).flex_none().text_right().child("Auto"))
}

fn connection_row(item: AetherItem, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    h_flex()
        .items_center()
        .h(px(40.0))
        .px(px(16.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.55))
        .hover(|this| this.bg(theme.list_hover))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(px(9.0))
                .child(
                    material_icon(item.icon)
                        .with_size(px(16.0))
                        .text_color(status_color(&item.kind, theme)),
                )
                .child(
                    div()
                        .min_w_0()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(item.name),
                ),
        )
        .child(
            div()
                .w(px(110.0))
                .flex_none()
                .text_size(px(11.0))
                .text_color(theme.foreground)
                .child(item.label),
        )
        .child(platform_badge(item.tag, theme).w(px(92.0)).flex_none())
        .child(
            div()
                .w(px(170.0))
                .flex_none()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(item.sub),
        )
        .child(
            h_flex()
                .w(px(112.0))
                .flex_none()
                .items_center()
                .gap(px(7.0))
                .child(
                    div()
                        .size(px(7.0))
                        .rounded(px(4.0))
                        .bg(status_color(&item.kind, theme)),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(item.status),
                ),
        )
        .child(
            div().w(px(64.0)).flex_none().text_right().child(
                material_icon(if item.active {
                    "toggle_on"
                } else {
                    "toggle_off"
                })
                .with_size(px(20.0))
                .text_color(if item.active {
                    theme.success
                } else {
                    theme.muted_foreground
                }),
            ),
        )
}

fn policy_list(
    title: &'static str,
    icon: &'static str,
    items: Vec<String>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .gap(px(8.0))
        .child(
            h_flex()
                .items_center()
                .gap(px(7.0))
                .text_size(px(11.0))
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(material_icon(icon).with_size(px(14.0)))
                .child(title),
        )
        .children(items.into_iter().map(|item| {
            div()
                .min_w_0()
                .px(px(9.0))
                .py(px(5.0))
                .rounded(px(6.0))
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .font_family(theme.mono_font_family.clone())
                .text_size(px(10.5))
                .text_color(theme.muted_foreground)
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(item)
        }))
}

fn builder_section(
    title: &'static str,
    icon: &'static str,
    right_header: &'static str,
    items: Vec<AetherItem>,
    empty_text: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let count = items.len();
    v_flex()
        .flex_1()
        .min_h(px(180.0))
        .min_w_0()
        .overflow_hidden()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .child(section_header(title, icon, count, theme))
        .child(builder_table_header("Name", "Details", right_header, theme))
        .when(items.is_empty(), |this| {
            this.child(compact_empty_state(icon, empty_text, theme))
        })
        .children(items.into_iter().map(|item| builder_row(item, theme)))
}

fn builder_process_section(
    processes: Vec<AetherItem>,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    let process_count = processes.len();
    v_flex()
        .flex_1()
        .min_h(px(180.0))
        .overflow_hidden()
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .child(section_header(
            "Asset Processor Processes",
            "lan",
            process_count,
            theme,
        ))
        .child(builder_table_header("Endpoint", "Owner", "State", theme))
        .when(processes.is_empty(), |this| {
            this.child(compact_empty_state(
                "lan",
                "No current asset-processor or worker endpoints reported",
                theme,
            ))
        })
        .children(processes.into_iter().map(|item| builder_row(item, theme)))
}

fn builder_metric_tile(
    title: &'static str,
    icon: &'static str,
    count: usize,
    detail: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_1()
        .min_w_0()
        .items_center()
        .gap(px(10.0))
        .p(px(12.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
        .child(
            material_icon(icon)
                .with_size(px(18.0))
                .text_color(theme.accent),
        )
        .child(
            v_flex()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .child(title),
                )
                .child(
                    div()
                        .mt(px(2.0))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(13.0))
                        .font_semibold()
                        .text_color(theme.foreground)
                        .child(format!("{count}")),
                ),
        )
        .child(
            div()
                .ml_auto()
                .max_w(px(150.0))
                .text_right()
                .text_size(px(10.5))
                .text_color(theme.muted_foreground)
                .child(detail),
        )
}

fn section_header(
    title: &'static str,
    icon: &'static str,
    count: usize,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(8.0))
        .px(px(12.0))
        .h(px(36.0))
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.tab_bar)
        .child(
            material_icon(icon)
                .with_size(px(15.0))
                .text_color(theme.muted_foreground),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(11.0))
                .font_semibold()
                .text_color(theme.foreground)
                .child(title),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(11.0))
                .text_color(theme.muted_foreground)
                .child(format!("{count}")),
        )
}

fn builder_table_header(
    primary: &'static str,
    secondary: &'static str,
    trailing: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .flex_none()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .h(px(28.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.7))
        .bg(theme.background.opacity(0.35))
        .font_family(theme.mono_font_family.clone())
        .text_size(px(10.0))
        .text_color(theme.muted_foreground)
        .child(div().w(px(18.0)).flex_none())
        .child(div().w(px(160.0)).flex_none().child(primary))
        .child(div().flex_1().min_w_0().child(secondary))
        .child(div().w(px(82.0)).flex_none().text_right().child(trailing))
}

fn compact_empty_state(
    icon: &'static str,
    text: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_h(px(120.0))
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .p(px(20.0))
        .text_color(theme.muted_foreground)
        .child(
            material_icon(icon)
                .with_size(px(28.0))
                .text_color(theme.border),
        )
        .child(div().text_size(px(12.0)).child(text))
}

fn builder_row(item: AetherItem, theme: &gpui_component::theme::Theme) -> impl IntoElement {
    let trailing = if !item.count.is_empty() {
        item.count
    } else if !item.status.is_empty() {
        item.status
    } else {
        item.tag
    };
    h_flex()
        .items_center()
        .gap(px(12.0))
        .px(px(12.0))
        .py(px(10.0))
        .border_b_1()
        .border_color(theme.border.opacity(0.55))
        .hover(|this| this.bg(theme.list_hover))
        .child(
            material_icon(item.icon)
                .with_size(px(18.0))
                .text_color(theme.accent),
        )
        .child(
            div()
                .w(px(160.0))
                .flex_none()
                .font_semibold()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(item.name),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(8.0))
                        .when(!item.label.is_empty(), |this| {
                            this.child(platform_badge(item.label, theme))
                        }),
                )
                .child(
                    div()
                        .mt(px(3.0))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(11.0))
                        .text_color(theme.muted_foreground)
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .child(item.sub),
                ),
        )
        .when(!trailing.is_empty(), |this| {
            this.child(
                div()
                    .w(px(82.0))
                    .flex_none()
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(11.0))
                    .text_color(theme.muted_foreground)
                    .child(trailing),
            )
        })
}

fn empty_state(
    icon: &'static str,
    text: &'static str,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap(px(8.0))
        .p(px(40.0))
        .text_color(theme.muted_foreground)
        .child(
            material_icon(icon)
                .with_size(px(34.0))
                .text_color(theme.border),
        )
        .child(div().text_size(px(12.0)).child(text))
}

fn status_color(kind: &str, theme: &gpui_component::theme::Theme) -> gpui::Hsla {
    match kind {
        "active" => theme.accent,
        "queued" => theme.muted_foreground,
        "succeeded" => theme.success,
        "warning" => theme.warning,
        "failed" | "error" => theme.danger,
        _ => theme.muted_foreground,
    }
}

fn status_icon_for_kind(kind: &str) -> &'static str {
    match kind {
        "active" => "sync",
        "queued" => "schedule",
        "succeeded" => "check_circle",
        "warning" => "warning",
        "failed" | "error" => "error",
        _ => "inventory_2",
    }
}

fn latest_job_for_source(
    cx: &mut Context<AetherAssetProcessorView>,
    source_key: &str,
) -> Option<(i64, Option<i64>)> {
    cx.try_global::<EditorAssetBrowserStatus>()
        .and_then(|status| {
            status
                .entries
                .iter()
                .find(|entry| format!("{}:{}", entry.root_id, entry.source_path) == source_key)
        })
        .and_then(|entry| entry.latest_job.as_ref())
        .map(|job| (job.job_id, job.attempt_id))
}
