//! Build, asset activity, console, output, profiler, and runtime-preview projections.

use std::path::Path;

use az_editor_ui::panels::asset_creation::{
    AssetSourceCreateRequestData, build_asset_source_create_request,
};
use az_editor_ui::panels::{
    AssetBrowserJobStatus, ConsoleLevelCounts, ConsoleState, EditorAssetBrowserStatus,
    EditorAssetProcessorActivity, EditorGpuStateData, EditorGpuStatus, EditorRuntimeStateData,
    EditorRuntimeStatus, EditorSessionStateData, EditorSessionStatus,
    EditorViewportRenderStateData, EditorViewportRenderStatus, EditorViewportTelemetryData,
    LogLevel, LogMessage, OutputLogMessage, OutputLogState, ProfilerPipelineStatus,
    SessionProcessStateData, format_console_time, gpu_pipeline_status, project_workflow,
    runtime_pipeline_status, validate_asset_db_relative_path, viewport_pipeline_status,
};
use az_editor_ui::{
    EditorBuildProfileData, EditorBuildTargetData, EditorProjectBuildCatalog,
    EditorProjectBuildPhase, EditorProjectBuildState,
};
use gpui::{AppContext, Context, Hsla, Window};
use gpui_component::ActiveTheme;

use crate::app::aether_common::{AetherItem, AetherStyle};
use crate::app::aether_editor_model::AetherConsoleFilter;
use crate::app::aether_editor_view::AetherEditorView;

use super::AetherViewAction;
use crate::attach::EditorAttachSession;

use super::asset_browser::visible_asset_entries;
use super::presentation::{
    aether_tab_item, bottom_tab_badge_style, format_count, hsla_css, non_empty_string_or,
    plural_count, set_item_style, toolbar_segment_style,
};
use super::workspace_overlay::menus_settings::source_control_projection;

impl AetherEditorView {
    pub(crate) fn build_actions(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>() else {
            return Vec::new();
        };
        let Some(state) = cx.try_global::<EditorProjectBuildState>() else {
            return Vec::new();
        };
        if !state.can_plan(catalog) {
            return Vec::new();
        }

        let theme = cx.theme().clone();
        let busy = state.phase.is_busy();
        let mut item = AetherItem {
            kind: "build-action".to_owned(),
            key: "execute-build".to_owned(),
            label: if busy {
                "Building...".to_owned()
            } else {
                "Build".to_owned()
            },
            icon: if busy {
                "progress_activity".to_owned()
            } else {
                "build".to_owned()
            },
            shortcut: if busy {
                String::new()
            } else {
                "Ctrl+B".to_owned()
            },
            icon_color: hsla_css(if busy { theme.warning } else { theme.accent }),
            ..AetherItem::default()
        };
        set_item_style(&mut item, "style", build_menu_action_style(&theme, busy));
        vec![item]
    }
    pub(crate) fn bottom_tabs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let counts = cx
            .try_global::<ConsoleState>()
            .map(ConsoleState::level_counts)
            .unwrap_or_default();
        let theme = cx.theme().clone();
        let mut tabs = aether_bottom_tabs(counts, &theme);
        self.apply_collection_state("bottomTabs", &mut tabs, &theme);
        tabs
    }
    pub(crate) fn configs(&self) -> Vec<AetherItem> {
        Vec::new()
    }
    pub(crate) fn build_configs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>() else {
            return Vec::new();
        };
        let state = cx
            .try_global::<EditorProjectBuildState>()
            .cloned()
            .unwrap_or_else(|| EditorProjectBuildState::from_catalog(catalog));
        let selected = state
            .selected_profile(catalog)
            .map(|profile| profile.name.as_str());
        let theme = cx.theme().clone();
        catalog
            .profiles
            .iter()
            .map(|profile| build_profile_item(profile, selected, &theme))
            .collect()
    }
    pub(crate) fn console_entries(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(console) = cx.try_global::<ConsoleState>() else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        let diagnostics = self.state.diagnostics_presentation();
        console
            .messages()
            .iter()
            .rev()
            .filter(|message| diagnostics.console.filter.shows(message.level))
            .filter(|message| message.matches_query(diagnostics.console.query))
            .enumerate()
            .map(|(index, message)| console_entry_item(index, message, &theme))
            .collect()
    }
    pub(crate) fn console_query(&self) -> String {
        self.state
            .diagnostics_presentation()
            .console
            .query
            .to_owned()
    }
    pub(crate) fn console_filters(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(console) = cx.try_global::<ConsoleState>() else {
            return Vec::new();
        };
        let counts = console.level_counts();
        let theme = cx.theme().clone();
        let diagnostics = self.state.diagnostics_presentation();
        AetherConsoleFilter::ALL
            .iter()
            .map(|filter| {
                console_filter_item(
                    *filter,
                    counts,
                    diagnostics.console.filter == *filter,
                    &theme,
                )
            })
            .collect()
    }
    pub(crate) fn output_lines(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        cx.try_global::<OutputLogState>()
            .map_or_else(Vec::new, |state| {
                state
                    .messages()
                    .iter()
                    .enumerate()
                    .map(|(index, message)| output_log_item(index, message))
                    .collect()
            })
    }
    pub(crate) fn pipe_jobs(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let theme = cx.theme().clone();
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() {
            return self.pipe_jobs_from_asset_status(status, &theme);
        }
        Vec::new()
    }
    pub(crate) fn prof_bars(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let runtime = cx.try_global::<EditorRuntimeStatus>();
        let gpu = cx.try_global::<EditorGpuStatus>();
        let viewport = cx.try_global::<EditorViewportRenderStatus>();
        if runtime.is_none() && gpu.is_none() && viewport.is_none() {
            return Vec::new();
        }

        let theme = cx.theme().clone();
        [
            (
                "profiler-runtime",
                "Runtime",
                runtime_pipeline_status(runtime, &theme),
            ),
            ("profiler-gpu", "GPU", gpu_pipeline_status(gpu, &theme)),
            (
                "profiler-viewport",
                "Viewport",
                viewport_pipeline_status(viewport, &theme),
            ),
        ]
        .into_iter()
        .map(|(key, label, status)| profiler_bar_item(key, label, status))
        .collect()
    }
    pub(crate) fn prof_stats(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(viewport) = cx.try_global::<EditorViewportRenderStatus>() else {
            return Vec::new();
        };
        let Some(telemetry) = viewport.telemetry.as_ref() else {
            return Vec::new();
        };

        let theme = cx.theme().clone();
        profiler_stat_items(telemetry, &theme)
    }
    pub(crate) fn sub_items(&self) -> Vec<AetherItem> {
        Vec::new()
    }
    pub(crate) fn targets(&self) -> Vec<AetherItem> {
        Vec::new()
    }
    pub(crate) fn build_targets(&self, cx: &mut Context<Self>) -> Vec<AetherItem> {
        let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>() else {
            return Vec::new();
        };
        let state = cx
            .try_global::<EditorProjectBuildState>()
            .cloned()
            .unwrap_or_else(|| EditorProjectBuildState::from_catalog(catalog));
        let selected = state
            .selected_target(catalog)
            .map(|target| target.key.as_str());
        let theme = cx.theme().clone();
        catalog
            .targets
            .iter()
            .map(|target| build_target_item(target, selected, &theme))
            .collect()
    }
    pub(crate) fn build_seg_style(&self) -> AetherStyle {
        toolbar_segment_style(false, 6)
    }
    pub(crate) fn pipe_seg_style(&self) -> AetherStyle {
        toolbar_segment_style(true, 6)
    }
    pub(crate) fn is_playing(&self, cx: &mut Context<Self>) -> bool {
        runtime_is_playing(cx.try_global::<EditorRuntimeStatus>())
    }
    pub(crate) fn build_open(&self) -> bool {
        self.bool_value("buildOpen")
    }
    pub(crate) fn pipe_active(&self, cx: &mut Context<Self>) -> bool {
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() {
            return status.entries.iter().any(|entry| {
                matches!(
                    entry.latest_job.as_ref().map(|job| job.status),
                    Some(AssetBrowserJobStatus::Queued | AssetBrowserJobStatus::Leased)
                )
            });
        }
        false
    }
    pub(crate) fn pipe_open(&self) -> bool {
        self.bool_value("pipeOpen")
    }
    pub(crate) fn tab_console(&self) -> bool {
        self.bool_value("tabConsole")
    }
    pub(crate) fn tab_output(&self) -> bool {
        self.bool_value("tabOutput")
    }
    pub(crate) fn tab_profiler(&self) -> bool {
        self.bool_value("tabProfiler")
    }
    pub(crate) fn gpu_adapter_label(&self, cx: &mut Context<Self>) -> String {
        if let Some(gpu) = cx.try_global::<EditorGpuStatus>() {
            return gpu
                .adapter_name
                .clone()
                .unwrap_or_else(|| gpu_status_label(gpu));
        }
        "GPU".to_owned()
    }
    pub(crate) fn about_renderer_label(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorGpuStatus>()
            .and_then(|gpu| gpu.backend.clone())
            .unwrap_or_else(|| "Renderer not connected".to_owned())
    }
    pub(crate) fn about_gpu_adapter_label(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<EditorGpuStatus>()
            .and_then(|gpu| gpu.adapter_name.clone())
            .unwrap_or_else(|| "GPU not connected".to_owned())
    }
    pub(crate) fn build_label(&self, cx: &mut Context<Self>) -> String {
        if let (Some(state), Some(catalog)) = (
            cx.try_global::<EditorProjectBuildState>(),
            cx.try_global::<EditorProjectBuildCatalog>(),
        ) && let Some((label, _, _)) =
            crate::project_build::build_status_line_from_state(state, catalog)
        {
            return label;
        }
        build_status_label(
            cx.try_global::<project_workflow::Status>(),
            cx.try_global::<OutputLogState>(),
            cx.try_global::<EditorSessionStatus>(),
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<EditorAssetProcessorActivity>(),
        )
    }
    pub(crate) fn build_sub(&self, cx: &mut Context<Self>) -> String {
        if let (Some(state), Some(catalog)) = (
            cx.try_global::<EditorProjectBuildState>(),
            cx.try_global::<EditorProjectBuildCatalog>(),
        ) && let Some((_, sub, _)) =
            crate::project_build::build_status_line_from_state(state, catalog)
        {
            return sub;
        }
        build_status_summary(
            cx.try_global::<project_workflow::Status>(),
            cx.try_global::<OutputLogState>(),
            cx.try_global::<EditorSessionStatus>(),
            cx.try_global::<EditorAssetBrowserStatus>(),
            cx.try_global::<EditorAssetProcessorActivity>(),
        )
    }
    pub(crate) fn build_tooltip(&self, cx: &mut Context<Self>) -> String {
        if let (Some(state), Some(catalog)) = (
            cx.try_global::<EditorProjectBuildState>(),
            cx.try_global::<EditorProjectBuildCatalog>(),
        ) && let Some((label, sub, _)) =
            crate::project_build::build_status_line_from_state(state, catalog)
        {
            return format!("{label} - {sub}");
        }
        "Compiler - open Output Log".to_owned()
    }
    pub(crate) fn config_short(&self) -> String {
        self.string_value("buildConfigShort")
    }
    pub(crate) fn build_config_short(&self, cx: &mut Context<Self>) -> String {
        let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>() else {
            return "No profile".to_owned();
        };
        cx.try_global::<EditorProjectBuildState>()
            .and_then(|state| state.selected_profile(catalog))
            .map(|profile| compact_build_profile_label(&profile.name))
            .unwrap_or_else(|| "No profile".to_owned())
    }
    pub(crate) fn build_busy(&self, cx: &mut Context<Self>) -> bool {
        cx.try_global::<EditorProjectBuildState>()
            .is_some_and(|state| state.phase.is_busy())
    }
    pub(crate) fn build_button_label(&self, cx: &mut Context<Self>) -> String {
        if self.build_busy(cx) {
            "Building".to_owned()
        } else {
            "Build".to_owned()
        }
    }
    pub(crate) fn build_button_icon(&self, cx: &mut Context<Self>) -> String {
        if self.build_busy(cx) {
            "progress_activity".to_owned()
        } else {
            "build".to_owned()
        }
    }
    pub(crate) fn err_count(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<ConsoleState>()
            .map_or(0, |state| state.level_counts().error)
            .to_string()
    }
    pub(crate) fn pipe_badge(&self, cx: &mut Context<Self>) -> String {
        if self.project_host_connecting(cx) {
            return "opening".to_owned();
        }
        let activity = cx.try_global::<EditorAssetProcessorActivity>().cloned();
        if let Some(activity) = activity.as_ref()
            && asset_processor_activity_needs_attention(activity)
        {
            return asset_processor_activity_badge(activity);
        }
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() {
            let counts = asset_pipeline_counts(status);
            if counts.active > 0 {
                return counts.active.to_string();
            }
            if counts.failed > 0 {
                return format!("{} failed", counts.failed);
            }
            return counts.succeeded.to_string();
        }
        if let Some(activity) = activity.as_ref() {
            return asset_processor_activity_badge(activity);
        }
        "not connected".to_owned()
    }
    pub(crate) fn pipe_icon(&self) -> String {
        self.string_value("pipeIcon")
    }
    pub(crate) fn pipe_summary(&self, cx: &mut Context<Self>) -> String {
        if self.project_host_connecting(cx) {
            return "Project services are still opening".to_owned();
        }
        let activity = cx.try_global::<EditorAssetProcessorActivity>().cloned();
        if let Some(activity) = activity.as_ref()
            && asset_processor_activity_needs_attention(activity)
        {
            return asset_processor_activity_summary(activity);
        }
        if let Some(status) = cx.try_global::<EditorAssetBrowserStatus>() {
            let counts = asset_pipeline_counts(status);
            return format!(
                "{} active · {} failed · {} done",
                counts.active, counts.failed, counts.succeeded
            );
        }
        if let Some(activity) = activity.as_ref() {
            return asset_processor_activity_summary(activity);
        }
        "asset processor not connected".to_owned()
    }
    pub(crate) fn has_session_status(&self, cx: &mut Context<Self>) -> bool {
        cx.try_global::<EditorAttachSession>()
            .and_then(source_control_projection)
            .is_some()
    }
    pub(crate) fn stat_draws(&self, cx: &mut Context<Self>) -> String {
        if let Some(draw_calls) = cx
            .try_global::<EditorViewportRenderStatus>()
            .and_then(|status| status.telemetry.as_ref())
            .and_then(|telemetry| telemetry.draw_calls)
        {
            return format_count(draw_calls);
        }
        unavailable_stat()
    }
    pub(crate) fn stat_fps(&self, cx: &mut Context<Self>) -> String {
        if let Some(fps) = cx
            .try_global::<EditorViewportRenderStatus>()
            .and_then(|status| status.telemetry.as_ref())
            .and_then(|telemetry| telemetry.fps)
        {
            return format!("{fps} fps");
        }
        unavailable_stat()
    }
    pub(crate) fn stat_ms(&self, cx: &mut Context<Self>) -> String {
        if let Some(frame_time_us) = cx
            .try_global::<EditorViewportRenderStatus>()
            .and_then(|status| status.telemetry.as_ref())
            .and_then(|telemetry| telemetry.frame_time_us)
        {
            return format!("{} ms", format_millis_from_micros(frame_time_us));
        }
        unavailable_stat()
    }
    pub(crate) fn stat_tris(&self, cx: &mut Context<Self>) -> String {
        if let Some(triangles) = cx
            .try_global::<EditorViewportRenderStatus>()
            .and_then(|status| status.telemetry.as_ref())
            .and_then(|telemetry| telemetry.triangles)
        {
            return format_count(triangles);
        }
        unavailable_stat()
    }
    pub(crate) fn stat_verts(&self, cx: &mut Context<Self>) -> String {
        if let Some(vertices) = cx
            .try_global::<EditorViewportRenderStatus>()
            .and_then(|status| status.telemetry.as_ref())
            .and_then(|telemetry| telemetry.vertices)
        {
            return format_count(vertices);
        }
        unavailable_stat()
    }
    pub(crate) fn stat_vram(&self, cx: &mut Context<Self>) -> String {
        if let Some(gpu_memory_bytes) = cx
            .try_global::<EditorViewportRenderStatus>()
            .and_then(|status| status.telemetry.as_ref())
            .and_then(|telemetry| telemetry.gpu_memory_bytes)
        {
            return format_bytes(gpu_memory_bytes);
        }
        unavailable_stat()
    }
    pub(crate) fn warn_count(&self, cx: &mut Context<Self>) -> String {
        cx.try_global::<ConsoleState>()
            .map_or(0, |state| state.level_counts().warn)
            .to_string()
    }
    pub(crate) fn console_on_filter(
        &mut self,
        value: impl AsRef<str>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_input("console_on_filter", value.as_ref());
        cx.notify();
    }
    pub(crate) fn go_console<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::GoConsole);
        cx.notify();
    }
    pub(crate) fn go_output<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::GoOutput);
        cx.notify();
    }
    pub(crate) fn go_profiler<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::GoProfiler);
        cx.notify();
    }
    pub(crate) fn on_clear_console<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.default_global::<ConsoleState>().clear();
        self.state.clear_console_query_state();
        cx.stop_propagation();
        cx.notify();
    }
    pub(crate) fn toggle_build<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::ToggleBuild);
        cx.notify();
    }

    pub(crate) fn close_build<E>(
        &mut self,
        _event: &E,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_action(AetherViewAction::CloseBuild);
        cx.notify();
    }

    pub(super) fn build_asset_create_request(
        &self,
        cx: &mut Context<Self>,
    ) -> Result<AssetSourceCreateRequestData, az_editor_ui::panels::AssetSourceCreateRequestError>
    {
        let Some(source) = self.selected_asset_create_source(cx) else {
            return Err(az_editor_ui::panels::AssetSourceCreateRequestError::EmptyName);
        };
        let draft = self.state.asset_create_draft();
        build_asset_source_create_request(&source, draft.folder, draft.name)
    }

    pub(super) fn build_asset_rename_request(&self) -> Result<(String, String, String), String> {
        let draft = self.state.asset_rename_draft();
        let source_root = draft.source_root.trim();
        if source_root.is_empty() {
            return Err("asset source root is unavailable".to_owned());
        }
        let from_source_path =
            validate_asset_db_relative_path(draft.from_path).ok_or_else(|| {
                format!(
                    "source path `{}` must be a portable relative path",
                    draft.from_path
                )
            })?;
        let to_source_path =
            validate_asset_db_relative_path(draft.to_path.trim()).ok_or_else(|| {
                format!(
                    "new source path `{}` must be a portable relative path",
                    draft.to_path
                )
            })?;
        if from_source_path == to_source_path {
            return Err("enter a different source path".to_owned());
        }
        Ok((source_root.to_owned(), from_source_path, to_source_path))
    }

    pub(crate) fn build_dot_style(&self, cx: &mut Context<Self>) -> AetherStyle {
        let theme = cx.theme().clone();
        let phase = cx
            .try_global::<EditorProjectBuildState>()
            .map_or(EditorProjectBuildPhase::Idle, |state| state.phase);
        build_status_dot_style(phase, &theme)
    }
}

pub(crate) fn aether_bottom_tabs(
    counts: ConsoleLevelCounts,
    theme: &gpui_component::theme::Theme,
) -> Vec<AetherItem> {
    let console_badge = counts.warn + counts.error;
    [
        ("assets", "Asset Browser", "folder", 0usize),
        ("console", "Console", "terminal", console_badge),
        ("output", "Output Log", "subject", 0),
        ("profiler", "Profiler", "speed", 0),
        ("gems", "Gems", "extension", 0),
    ]
    .into_iter()
    .map(|(key, label, icon, badge)| {
        let mut item = aether_tab_item((key, label, icon));
        if badge > 0 {
            item.has_badge = true;
            item.badge = badge.to_string();
            set_item_style(&mut item, "badgeStyle", bottom_tab_badge_style(theme));
        }
        item
    })
    .collect()
}

fn build_profile_item(
    profile: &EditorBuildProfileData,
    selected_profile: Option<&str>,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let selected = selected_profile == Some(profile.name.as_str());
    let mut item = AetherItem {
        kind: "build-profile".to_owned(),
        key: profile.name.clone(),
        name: profile.name.clone(),
        sub: build_profile_summary(profile),
        active: selected,
        selected,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", build_menu_row_style(selected, theme));
    set_item_style(
        &mut item,
        "dotStyle",
        build_profile_dot_style(selected, theme),
    );
    item
}

fn build_target_item(
    target: &EditorBuildTargetData,
    selected_target: Option<&str>,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let selected = selected_target == Some(target.key.as_str());
    let mut item = AetherItem {
        kind: "build-target".to_owned(),
        key: target.key.clone(),
        name: target.label.clone(),
        icon: target.icon.clone(),
        active: selected,
        selected,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", build_menu_row_style(selected, theme));
    item
}

fn compact_build_profile_label(name: &str) -> String {
    name.rsplit_once('-')
        .map_or(name, |(_, suffix)| suffix)
        .to_owned()
}

fn build_profile_summary(profile: &EditorBuildProfileData) -> String {
    [
        profile.asset_platform.as_str(),
        profile.cargo_profile.as_str(),
        profile.container.as_str(),
        profile.compression.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

fn build_menu_row_style(selected: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "background",
        hsla_css(if selected {
            theme.accent.opacity(0.12)
        } else {
            theme.transparent
        }),
    )])
}

fn build_menu_action_style(theme: &gpui_component::theme::Theme, busy: bool) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("justifyContent", "space-between".to_owned()),
        ("height", "30px".to_owned()),
        ("padding", "0 10px".to_owned()),
        ("fontSize", "11.5px".to_owned()),
        (
            "cursor",
            if busy { "default" } else { "pointer" }.to_owned(),
        ),
        (
            "color",
            hsla_css(if busy {
                theme.muted_foreground
            } else {
                theme.foreground
            }),
        ),
    ])
}

fn build_profile_dot_style(selected: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[(
        "background",
        hsla_css(if selected {
            theme.accent
        } else {
            theme.muted_foreground
        }),
    )])
}

fn build_status_dot_style(
    phase: EditorProjectBuildPhase,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[("background", hsla_css(phase.tone().color(theme)))])
}

fn output_log_item(index: usize, message: &OutputLogMessage) -> AetherItem {
    AetherItem {
        key: format!("output-log-{index}"),
        text: message.message.clone(),
        ..AetherItem::default()
    }
}

fn console_filter_item(
    filter: AetherConsoleFilter,
    counts: ConsoleLevelCounts,
    active: bool,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        key: filter.key().to_owned(),
        label: filter.label().to_owned(),
        icon: filter.icon().to_owned(),
        color: hsla_css(console_filter_icon_color(filter, active, theme)),
        count: filter.count_from(counts).to_string(),
        active,
        selected: active,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", console_filter_chip_style(active, theme));
    item
}

fn console_entry_item(
    index: usize,
    message: &LogMessage,
    theme: &gpui_component::theme::Theme,
) -> AetherItem {
    let mut item = AetherItem {
        key: format!("console-entry-{index}"),
        icon: console_level_icon(message.level).to_owned(),
        color: hsla_css(console_level_icon_color(message.level, theme)),
        time: format_console_time(message.timestamp),
        src: non_empty_string_or(&message.source, "editor"),
        msg: message.message.clone(),
        ..AetherItem::default()
    };
    set_item_style(&mut item, "style", console_entry_row_style());
    set_item_style(
        &mut item,
        "msgStyle",
        console_entry_message_style(message.level, theme),
    );
    item
}

const fn console_level_icon(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error => "error",
        LogLevel::Warn => "warning",
        LogLevel::Info => "info",
        LogLevel::Debug | LogLevel::Trace => "chat_bubble",
    }
}

fn console_level_icon_color(level: LogLevel, theme: &gpui_component::theme::Theme) -> Hsla {
    match level {
        LogLevel::Error => theme.danger,
        LogLevel::Warn => theme.warning,
        LogLevel::Info => theme.info,
        LogLevel::Debug | LogLevel::Trace => theme.muted_foreground,
    }
}

fn console_filter_icon_color(
    filter: AetherConsoleFilter,
    active: bool,
    theme: &gpui_component::theme::Theme,
) -> Hsla {
    if !active {
        return theme.muted_foreground;
    }
    match filter {
        AetherConsoleFilter::All => theme.muted_foreground,
        AetherConsoleFilter::Info => theme.info,
        AetherConsoleFilter::Warn => theme.warning,
        AetherConsoleFilter::Error => theme.danger,
    }
}

fn console_entry_message_style(
    level: LogLevel,
    theme: &gpui_component::theme::Theme,
) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("flex", "1 1 auto".to_owned()),
        (
            "color",
            hsla_css(match level {
                LogLevel::Error => theme.danger,
                LogLevel::Warn => theme.warning,
                LogLevel::Info => theme.foreground,
                LogLevel::Debug | LogLevel::Trace => theme.muted_foreground,
            }),
        ),
    ])
}

fn console_entry_row_style() -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "8px".to_owned()),
        ("minHeight", "22px".to_owned()),
        ("padding", "2px 10px".to_owned()),
    ])
}

fn console_filter_chip_style(active: bool, theme: &gpui_component::theme::Theme) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("display", "flex".to_owned()),
        ("alignItems", "center".to_owned()),
        ("gap", "5px".to_owned()),
        ("height", "23px".to_owned()),
        ("padding", "0 9px".to_owned()),
        ("borderRadius", "5px".to_owned()),
        ("fontSize", "11px".to_owned()),
        ("cursor", "default".to_owned()),
        (
            "color",
            hsla_css(if active {
                theme.foreground
            } else {
                theme.muted_foreground
            }),
        ),
        (
            "background",
            if active {
                hsla_css(theme.secondary)
            } else {
                "transparent".to_owned()
            },
        ),
    ])
}

fn profiler_stat_items(
    telemetry: &EditorViewportTelemetryData,
    theme: &gpui_component::theme::Theme,
) -> Vec<AetherItem> {
    let mut items = Vec::new();
    if let Some(fps) = telemetry.fps {
        items.push(profiler_stat_item(
            "profiler-stat-fps",
            "FPS",
            fps.to_string(),
            theme.success,
        ));
    }
    if let Some(frame_time_us) = telemetry.frame_time_us {
        items.push(profiler_stat_item(
            "profiler-stat-frame",
            "Frame",
            format!("{} ms", format_millis_from_micros(frame_time_us)),
            theme.foreground,
        ));
    }
    if let Some(draw_calls) = telemetry.draw_calls {
        items.push(profiler_stat_item(
            "profiler-stat-draw-calls",
            "Draw Calls",
            format_count(draw_calls),
            theme.foreground,
        ));
    }
    if let Some(triangles) = telemetry.triangles {
        items.push(profiler_stat_item(
            "profiler-stat-triangles",
            "Triangles",
            format_count(triangles),
            theme.foreground,
        ));
    }
    if let Some(vertices) = telemetry.vertices {
        items.push(profiler_stat_item(
            "profiler-stat-vertices",
            "Vertices",
            format_count(vertices),
            theme.foreground,
        ));
    }
    if let Some(gpu_memory_bytes) = telemetry.gpu_memory_bytes {
        items.push(profiler_stat_item(
            "profiler-stat-vram",
            "VRAM",
            format_bytes(gpu_memory_bytes),
            theme.warning,
        ));
    }
    items
}

fn profiler_stat_item(key: &str, label: &str, value: String, color: Hsla) -> AetherItem {
    AetherItem {
        key: key.to_owned(),
        label: label.to_owned(),
        value,
        color: hsla_css(color),
        ..AetherItem::default()
    }
}

fn profiler_bar_item(key: &str, label: &str, status: ProfilerPipelineStatus) -> AetherItem {
    let bar_style = profiler_bar_style(&status);
    let mut item = AetherItem {
        key: key.to_owned(),
        label: label.to_owned(),
        ms: status.label,
        ..AetherItem::default()
    };
    set_item_style(&mut item, "barStyle", bar_style);
    item
}

fn profiler_bar_style(status: &ProfilerPipelineStatus) -> AetherStyle {
    AetherStyle::from_pairs(&[
        ("height", "100%".to_owned()),
        (
            "width",
            format!("{:.0}%", status.progress.clamp(0.0, 1.0) * 100.0),
        ),
        ("background", hsla_css(status.color)),
        ("borderRadius", "3px".to_owned()),
    ])
}

fn unavailable_stat() -> String {
    "—".to_owned()
}

fn output_log_level_count(cx: &mut Context<AetherEditorView>, level: LogLevel) -> usize {
    cx.try_global::<OutputLogState>().map_or(0, |state| {
        state
            .messages()
            .iter()
            .filter(|message| message.level == level)
            .count()
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AssetPipelineCounts {
    pub(crate) active: usize,
    pub(crate) failed: usize,
    pub(crate) succeeded: usize,
}

pub(crate) fn asset_pipeline_counts(status: &EditorAssetBrowserStatus) -> AssetPipelineCounts {
    let mut counts = AssetPipelineCounts::default();
    for job in status
        .entries
        .iter()
        .filter_map(|entry| entry.latest_job.as_ref())
    {
        match job.status {
            AssetBrowserJobStatus::Queued | AssetBrowserJobStatus::Leased => counts.active += 1,
            AssetBrowserJobStatus::Failed | AssetBrowserJobStatus::Abandoned => {
                counts.failed += 1;
            }
            AssetBrowserJobStatus::Succeeded => counts.succeeded += 1,
        }
    }
    counts
}

pub(crate) fn build_status_label(
    workflow: Option<&project_workflow::Status>,
    output: Option<&OutputLogState>,
    session: Option<&EditorSessionStatus>,
    asset: Option<&EditorAssetBrowserStatus>,
    asset_activity: Option<&EditorAssetProcessorActivity>,
) -> String {
    if let Some(status) = workflow
        && project_workflow_status_needs_status_line_attention(status)
    {
        return project_workflow_status_label(status);
    }
    if let Some(label) = output.and_then(output_log_status_label) {
        return label;
    }
    if asset.is_some_and(asset_status_needs_status_line_attention) {
        return "Assets".to_owned();
    }
    if asset_activity.is_some_and(asset_processor_activity_needs_attention) {
        return "Assets".to_owned();
    }
    if session.is_some() {
        return "Session".to_owned();
    }
    if asset.is_some() {
        return "Assets".to_owned();
    }
    if asset_activity.is_some() {
        return "Assets".to_owned();
    }
    if let Some(status) = workflow
        && status.attached_session.is_some()
    {
        return "Session".to_owned();
    }
    "Project".to_owned()
}

pub(crate) fn build_status_summary(
    workflow: Option<&project_workflow::Status>,
    output: Option<&OutputLogState>,
    session: Option<&EditorSessionStatus>,
    asset: Option<&EditorAssetBrowserStatus>,
    asset_activity: Option<&EditorAssetProcessorActivity>,
) -> String {
    if let Some(status) = workflow
        && project_workflow_status_needs_status_line_attention(status)
    {
        return project_workflow_status_summary(status);
    }
    if let Some(summary) = output.and_then(output_log_status_summary) {
        return summary;
    }
    if let Some(status) = asset
        && asset_status_needs_status_line_attention(status)
    {
        return asset_status_summary(status);
    }
    if let Some(activity) = asset_activity
        && asset_processor_activity_needs_attention(activity)
    {
        return asset_processor_activity_summary(activity);
    }
    if let Some(status) = session {
        return session_status_summary(status);
    }
    if let Some(status) = asset {
        return asset_status_summary(status);
    }
    if let Some(activity) = asset_activity {
        return asset_processor_activity_summary(activity);
    }
    if let Some(status) = workflow
        && let Some(attached) = &status.attached_session
    {
        return format!(
            "attached · {}",
            plural_count(attached.running_service_names.len(), "service")
        );
    }
    "not attached".to_owned()
}

fn output_log_status_label(output: &OutputLogState) -> Option<String> {
    output.latest().map(|message| {
        message
            .source
            .split_once(':')
            .map_or(message.source.as_str(), |(service, _)| service)
            .to_owned()
    })
}

fn output_log_status_summary(output: &OutputLogState) -> Option<String> {
    output.latest().map(|message| {
        format!(
            "{} · {}",
            status_log_level_label(message.level),
            message.message
        )
    })
}

const fn status_log_level_label(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace => "trace",
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
    }
}

fn project_workflow_status_needs_status_line_attention(status: &project_workflow::Status) -> bool {
    matches!(
        status.phase,
        project_workflow::Phase::Running | project_workflow::Phase::Failed
    ) || status.progress.is_some()
}

fn project_workflow_status_label(status: &project_workflow::Status) -> String {
    if status.status_error.is_some() {
        return "Project".to_owned();
    }
    if let Some(progress) = &status.progress {
        return progress.phase.label().to_owned();
    }
    status.operation.map_or_else(
        || "Project".to_owned(),
        |operation| project_workflow_operation_label(operation).to_owned(),
    )
}

fn project_workflow_status_summary(status: &project_workflow::Status) -> String {
    if let Some(error) = &status.status_error {
        return format!("error: {error}");
    }
    if let Some(progress) = &status.progress {
        return project_workflow_progress_summary(progress);
    }
    status
        .message
        .clone()
        .unwrap_or_else(|| "running".to_owned())
}

fn project_workflow_progress_summary(progress: &project_workflow::Progress) -> String {
    let message = if progress.message.trim().is_empty() {
        progress.phase.label()
    } else {
        progress.message.as_str()
    };
    progress.phase_total.map_or_else(
        || {
            if progress.phase_done > 0 {
                format!("live output · {} unit(s) · {message}", progress.phase_done)
            } else {
                format!("waiting for output · {message}")
            }
        },
        |total| {
            let percent = progress.phase_percent().unwrap_or(0);
            format!("{percent}% · {}/{} · {message}", progress.phase_done, total)
        },
    )
}

const fn project_workflow_operation_label(operation: project_workflow::Operation) -> &'static str {
    match operation {
        project_workflow::Operation::CreateProject => "Create project",
        project_workflow::Operation::InitializeProject => "Initialize project",
        project_workflow::Operation::EnsureProjectSession => "Session",
        project_workflow::Operation::PrepareProjectServices => "Services",
        project_workflow::Operation::OpenEditorSession => "Open project",
    }
}

fn asset_status_needs_status_line_attention(status: &EditorAssetBrowserStatus) -> bool {
    if status.status_error.is_some() {
        return true;
    }
    let counts = asset_pipeline_counts(status);
    counts.active > 0 || counts.failed > 0
}

fn asset_processor_activity_needs_attention(activity: &EditorAssetProcessorActivity) -> bool {
    activity.busy() || activity.degraded || !activity.ready
}

fn asset_processor_activity_badge(activity: &EditorAssetProcessorActivity) -> String {
    if activity.busy() {
        let operation = activity.operation.trim();
        if operation.eq_ignore_ascii_case("source-reconcile") {
            return "scanning".to_owned();
        }
        if !operation.is_empty() {
            return operation.replace('-', " ");
        }
        return "busy".to_owned();
    }
    if activity.degraded || !activity.ready {
        return "health".to_owned();
    }
    "ready".to_owned()
}

fn asset_processor_activity_summary(activity: &EditorAssetProcessorActivity) -> String {
    let message = activity.message.trim();
    if !message.is_empty() {
        return message.to_owned();
    }

    let state = activity.state.label();
    let operation = activity.operation.trim();
    if operation.is_empty() {
        state.to_owned()
    } else {
        format!("{state} · {}", operation.replace('-', " "))
    }
}

pub(crate) fn asset_status_summary(status: &EditorAssetBrowserStatus) -> String {
    if let Some(error) = &status.status_error {
        return format!("error: {error}");
    }
    let counts = asset_pipeline_counts(status);
    if counts.active > 0 || counts.failed > 0 {
        return format!(
            "{} active · {} failed · {} done",
            counts.active, counts.failed, counts.succeeded
        );
    }
    format!(
        "{} indexed",
        format_count(visible_asset_entries(status).count())
    )
}

pub(crate) fn session_status_summary(status: &EditorSessionStatus) -> String {
    let running = status
        .processes
        .iter()
        .filter(|process| process.state == SessionProcessStateData::Running)
        .count();
    let failed = status
        .processes
        .iter()
        .filter(|process| process.state == SessionProcessStateData::Failed)
        .count();
    let supervised = status.processes.len();
    let mut label = format!(
        "{} · {} services attached · {running}/{supervised} supervised",
        status.state.label(),
        status.services_count,
    );
    if failed > 0 {
        label.push_str(&format!(" · {failed} failed"));
    }
    if let Some(reason) = status
        .failure_reason
        .as_deref()
        .filter(|reason| !reason.is_empty())
    {
        label.push_str(" · ");
        label.push_str(reason);
    }
    label
}

fn format_millis_from_micros(micros: u32) -> String {
    let whole = micros / 1_000;
    let fraction = micros % 1_000;
    format!("{whole}.{fraction:03}")
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

fn runtime_is_playing(status: Option<&EditorRuntimeStatus>) -> bool {
    status.is_some_and(|status| {
        matches!(
            status.state,
            EditorRuntimeStateData::Starting | EditorRuntimeStateData::Running
        )
    })
}

pub(crate) fn runtime_play_icon(status: Option<&EditorRuntimeStatus>) -> &'static str {
    if runtime_is_playing(status) {
        "pause"
    } else {
        "play_arrow"
    }
}

fn viewport_render_label(status: &EditorViewportRenderStatus) -> String {
    match status.state {
        EditorViewportRenderStateData::Waiting => "waiting".to_owned(),
        EditorViewportRenderStateData::MetadataOnly => status.format.as_ref().map_or_else(
            || "metadata".to_owned(),
            |format| format!("metadata {format}"),
        ),
        EditorViewportRenderStateData::GpuSurfaceHandle => "surface".to_owned(),
        EditorViewportRenderStateData::EditorCompositionSurface => "composition-surface".to_owned(),
        EditorViewportRenderStateData::Failed => "failed".to_owned(),
    }
}

fn gpu_status_label(status: &EditorGpuStatus) -> String {
    match status.state {
        EditorGpuStateData::NotRequested => "gpu idle".to_owned(),
        EditorGpuStateData::Starting => "gpu starting".to_owned(),
        EditorGpuStateData::Ready => status
            .adapter_name
            .clone()
            .unwrap_or_else(|| "gpu ready".to_owned()),
        EditorGpuStateData::Failed => "gpu failed".to_owned(),
    }
}
