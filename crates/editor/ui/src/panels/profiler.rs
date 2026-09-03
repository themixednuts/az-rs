//! Profiler panel.
//!
//! Shows live editor/runtime pipeline health. Real frame timings can replace
//! these status-derived bars once the runtime publishes metric samples.

use gpui::{
    App, Context, FocusHandle, Focusable, Hsla, IntoElement, ParentElement, Render, Styled, Window,
    div, px, relative,
};
use gpui_component::dock::{Panel, PanelEvent};
use gpui_component::scroll::ScrollableElement;
use gpui_component::{ActiveTheme, StyledExt, h_flex, v_flex};

use crate::panels::{
    EditorGpuStateData, EditorGpuStatus, EditorRuntimeStateData, EditorRuntimeStatus,
    EditorViewportRenderStateData, EditorViewportRenderStatus, kit,
};
use crate::status::StatusTone;

pub struct ProfilerPanel {
    focus_handle: FocusHandle,
}

impl ProfilerPanel {
    pub const NAME: &'static str = "profiler";

    pub fn init(cx: &mut Context<'_, Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl Render for ProfilerPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let runtime = cx.try_global::<EditorRuntimeStatus>().cloned();
        let gpu = cx.try_global::<EditorGpuStatus>().cloned();
        let viewport = cx.try_global::<EditorViewportRenderStatus>().cloned();
        let diagnostics =
            profiler_diagnostic_count(runtime.as_ref(), gpu.as_ref(), viewport.as_ref());

        let runtime_status = runtime_pipeline_status(runtime.as_ref(), &theme);
        let gpu_status = gpu_pipeline_status(gpu.as_ref(), &theme);
        let viewport_status = viewport_pipeline_status(viewport.as_ref(), &theme);
        let backend = profiler_backend_label(gpu.as_ref(), viewport.as_ref());
        let frame_extent = profiler_frame_extent_label(viewport.as_ref());
        let generation = profiler_generation_label(viewport.as_ref());

        v_flex().size_full().bg(theme.sidebar).child(
            v_flex()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p(px(14.0))
                .gap(px(14.0))
                .child(
                    h_flex()
                        .gap(px(10.0))
                        .flex_wrap()
                        .child(profiler_stat_card(
                            "Runtime",
                            runtime_status.label.clone(),
                            runtime_status.color,
                            &theme,
                        ))
                        .child(profiler_stat_card(
                            "GPU",
                            gpu_status.label.clone(),
                            gpu_status.color,
                            &theme,
                        ))
                        .child(profiler_stat_card(
                            "Viewport",
                            viewport_status.label.clone(),
                            viewport_status.color,
                            &theme,
                        ))
                        .child(profiler_stat_card(
                            "Frame",
                            frame_extent,
                            theme.info,
                            &theme,
                        ))
                        .child(profiler_stat_card("Backend", backend, theme.accent, &theme))
                        .child(profiler_stat_card(
                            "Diagnostics",
                            diagnostics.to_string(),
                            if diagnostics == 0 {
                                theme.success
                            } else {
                                theme.warning
                            },
                            &theme,
                        )),
                )
                .child(
                    div()
                        .text_size(gpui::Rems(0.58))
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .child("PIPELINE BREAKDOWN - LIVE STATUS"),
                )
                .child(profiler_bar("Runtime", runtime_status, &theme))
                .child(profiler_bar("GPU", gpu_status, &theme))
                .child(profiler_bar("Viewport", viewport_status, &theme))
                .child(profiler_static_row("Frame generation", generation, &theme)),
        )
    }
}

impl Focusable for ProfilerPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for ProfilerPanel {
    fn panel_name(&self) -> &'static str {
        Self::NAME
    }

    fn title(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        kit::tab_title(Some("speed"), "Profiler", kit::TabTone::Default)
    }
}

impl gpui::EventEmitter<PanelEvent> for ProfilerPanel {}

#[derive(Clone)]
pub struct ProfilerPipelineStatus {
    pub label: String,
    pub progress: f32,
    pub color: Hsla,
}

fn profiler_stat_card(
    label: &'static str,
    value: impl Into<String>,
    color: Hsla,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w(px(110.0))
        .gap(px(4.0))
        .p(px(10.0))
        .rounded(px(6.0))
        .bg(theme.background)
        .border_1()
        .border_color(theme.border)
        .child(
            div()
                .text_size(gpui::Rems(0.56))
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(label.to_uppercase()),
        )
        .child(
            div()
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.96))
                .font_medium()
                .text_color(color)
                .child(value.into()),
        )
}

fn profiler_bar(
    label: &'static str,
    status: ProfilerPipelineStatus,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(10.0))
        .mb(px(6.0))
        .child(
            div()
                .w(px(92.0))
                .text_size(gpui::Rems(0.62))
                .text_color(theme.sidebar_foreground)
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .h(px(14.0))
                .rounded(px(3.0))
                .overflow_hidden()
                .bg(theme.background)
                .child(
                    div()
                        .h_full()
                        .w(relative(status.progress.clamp(0.0, 1.0)))
                        .bg(status.color),
                ),
        )
        .child(
            div()
                .w(px(84.0))
                .text_align(gpui::TextAlign::Right)
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.6))
                .text_color(theme.muted_foreground)
                .child(status.label),
        )
}

fn profiler_static_row(
    label: &'static str,
    value: String,
    theme: &gpui_component::theme::Theme,
) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap(px(10.0))
        .child(
            div()
                .w(px(92.0))
                .text_size(gpui::Rems(0.62))
                .text_color(theme.sidebar_foreground)
                .child(label),
        )
        .child(div().flex_1().h(px(1.0)).bg(theme.border))
        .child(
            div()
                .w(px(84.0))
                .text_align(gpui::TextAlign::Right)
                .font_family(theme.mono_font_family.clone())
                .text_size(gpui::Rems(0.6))
                .text_color(theme.muted_foreground)
                .child(value),
        )
}

#[must_use]
pub fn profiler_diagnostic_count(
    runtime: Option<&EditorRuntimeStatus>,
    gpu: Option<&EditorGpuStatus>,
    viewport: Option<&EditorViewportRenderStatus>,
) -> usize {
    runtime.map_or(0, |status| status.diagnostics.len())
        + usize::from(gpu.and_then(|status| status.diagnostic.as_ref()).is_some())
        + usize::from(
            viewport
                .and_then(|status| status.diagnostic.as_ref())
                .is_some(),
        )
}

#[must_use]
pub fn profiler_backend_label(
    gpu: Option<&EditorGpuStatus>,
    viewport: Option<&EditorViewportRenderStatus>,
) -> String {
    viewport
        .and_then(|status| status.backend.clone())
        .or_else(|| gpu.and_then(|status| status.backend.clone()))
        .unwrap_or_else(|| "none".to_string())
}

#[must_use]
pub fn profiler_frame_extent_label(status: Option<&EditorViewportRenderStatus>) -> String {
    status
        .and_then(|status| Some((status.width?, status.height?)))
        .map_or_else(
            || "waiting".to_string(),
            |(width, height)| format!("{width}x{height}"),
        )
}

#[must_use]
pub fn profiler_generation_label(status: Option<&EditorViewportRenderStatus>) -> String {
    status
        .and_then(|status| status.generation)
        .map_or_else(|| "-".to_string(), |generation| generation.to_string())
}

#[must_use]
pub fn runtime_pipeline_status(
    status: Option<&EditorRuntimeStatus>,
    theme: &gpui_component::theme::Theme,
) -> ProfilerPipelineStatus {
    let state = status.map(|status| status.state);
    ProfilerPipelineStatus {
        // The bar reads "none" for both an absent runtime and an unregistered
        // one: neither has a pipeline to report on.
        label: match state {
            Some(EditorRuntimeStateData::Unregistered) | None => "none".to_string(),
            Some(state) => state.label().to_string(),
        },
        progress: match state {
            Some(EditorRuntimeStateData::Running) => 0.92,
            Some(EditorRuntimeStateData::Starting) => 0.42,
            Some(EditorRuntimeStateData::Failed) => 1.0,
            Some(EditorRuntimeStateData::Stopped) => 0.16,
            Some(EditorRuntimeStateData::Unregistered) | None => 0.08,
        },
        color: state
            .map_or(StatusTone::Neutral, EditorRuntimeStateData::tone)
            .color(theme),
    }
}

#[must_use]
pub fn gpu_pipeline_status(
    status: Option<&EditorGpuStatus>,
    theme: &gpui_component::theme::Theme,
) -> ProfilerPipelineStatus {
    match status.map(|status| status.state) {
        Some(EditorGpuStateData::Ready) => ProfilerPipelineStatus {
            label: "ready".to_string(),
            progress: 0.86,
            color: StatusTone::Success.color(theme),
        },
        Some(EditorGpuStateData::Starting) => ProfilerPipelineStatus {
            label: "starting".to_string(),
            progress: 0.36,
            color: StatusTone::Warning.color(theme),
        },
        Some(EditorGpuStateData::Failed) => ProfilerPipelineStatus {
            label: "failed".to_string(),
            progress: 1.0,
            color: StatusTone::Danger.color(theme),
        },
        Some(EditorGpuStateData::NotRequested) | None => ProfilerPipelineStatus {
            label: "idle".to_string(),
            progress: 0.1,
            color: StatusTone::Neutral.color(theme),
        },
    }
}

#[must_use]
pub fn viewport_pipeline_status(
    status: Option<&EditorViewportRenderStatus>,
    theme: &gpui_component::theme::Theme,
) -> ProfilerPipelineStatus {
    match status.map(|status| status.state) {
        Some(
            EditorViewportRenderStateData::EditorCompositionSurface
            | EditorViewportRenderStateData::GpuSurfaceHandle,
        ) => ProfilerPipelineStatus {
            label: "streaming".to_string(),
            progress: 0.88,
            color: StatusTone::Success.color(theme),
        },
        Some(EditorViewportRenderStateData::MetadataOnly) => ProfilerPipelineStatus {
            label: "metadata".to_string(),
            progress: 0.46,
            color: StatusTone::Info.color(theme),
        },
        Some(EditorViewportRenderStateData::Failed) => ProfilerPipelineStatus {
            label: "failed".to_string(),
            progress: 1.0,
            color: StatusTone::Danger.color(theme),
        },
        Some(EditorViewportRenderStateData::Waiting) | None => ProfilerPipelineStatus {
            label: "waiting".to_string(),
            progress: 0.18,
            color: StatusTone::Neutral.color(theme),
        },
    }
}
