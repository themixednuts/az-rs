//! Project build catalog loading and daemon planning controller.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use az_editor_ui::{
    ConsoleState, EditorBuildOutputTargetData, EditorBuildProfileData, EditorBuildTargetData,
    EditorProjectBuildCatalog, EditorProjectBuildCommandData, EditorProjectBuildPhase,
    EditorProjectBuildPlanData, EditorProjectBuildProgressData, EditorProjectBuildState, LogLevel,
    OutputLogState,
    actions::{
        ExecuteProjectBuild, PlanProjectBuild, SetProjectBuildProfile, SetProjectBuildTarget,
    },
};
use az_proto_core::Endpoint;
use az_proto_daemon::{
    ProjectBuildExecutionResult, ProjectBuildPackageProfile, ProjectBuildPlan,
    ProjectBuildProgressEvent, daemon_capnp,
};
use gpui::App;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use toml::Value;
use tracing::{error, info, instrument, warn};

use crate::attach::EditorAttachSession;
use crate::controller_set::{self, ControllerFence};
use crate::daemon::AzDaemonClient;

const BUILD_LOG_SOURCE: &str = "build";

#[derive(Clone, Debug)]
pub struct EditorProjectBuildController {
    daemon_endpoint: Endpoint,
    project_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectBuildProgressUpdate {
    pub seq: u64,
    pub command_index: u32,
    pub command_count: u32,
    pub target_name: String,
    pub done_bp: u32,
    pub command_done: u64,
    pub command_total: Option<u64>,
    pub message: String,
}

impl ProjectBuildProgressUpdate {
    #[must_use]
    pub fn to_ui_data(&self) -> EditorProjectBuildProgressData {
        EditorProjectBuildProgressData {
            seq: self.seq,
            command_index: self.command_index,
            command_count: self.command_count,
            target_name: self.target_name.clone(),
            done_bp: self.done_bp,
            command_done: self.command_done,
            command_total: self.command_total,
            message: self.message.clone(),
        }
    }
}

pub struct EditorBuildProgressSink {
    tx: UnboundedSender<ProjectBuildProgressUpdate>,
}

impl EditorBuildProgressSink {
    #[must_use]
    pub const fn new(tx: UnboundedSender<ProjectBuildProgressUpdate>) -> Self {
        Self { tx }
    }

    #[must_use]
    pub fn into_client(self) -> daemon_capnp::project_build_progress_sink::Client {
        capnp_rpc::new_client(self)
    }
}

impl daemon_capnp::project_build_progress_sink::Server for EditorBuildProgressSink {
    #[allow(clippy::future_not_send)] // capnp-rpc promises are `!Send`; driven on GPUI's foreground executor.
    async fn update(
        self: capnp::capability::Rc<Self>,
        params: daemon_capnp::project_build_progress_sink::UpdateParams,
        _results: daemon_capnp::project_build_progress_sink::UpdateResults,
    ) -> Result<(), capnp::Error> {
        let event = ProjectBuildProgressEvent::from_capnp(params.get()?.get_event()?)?;
        let update = ProjectBuildProgressUpdate {
            seq: event.seq,
            command_index: event.command_index,
            command_count: event.command_count,
            target_name: event.target_name,
            done_bp: event.done_bp,
            command_done: event.command_done,
            command_total: (event.command_total != 0).then_some(event.command_total),
            message: event.message,
        };
        let _ = self.tx.send(update);
        Ok(())
    }
}

#[instrument(skip(cx, session), fields(project_id = %session.project_id, session_slug = %session.session_slug))]
pub(crate) fn install_project_build_slot(
    cx: &mut App,
    session: EditorAttachSession,
    fence: ControllerFence,
) {
    let catalog = load_project_build_catalog(&session.project_root);
    let profile_count = catalog.profiles.len();
    let output_target_count = catalog.default_build_targets.len();
    let diagnostic_count = catalog.diagnostics.len();
    let state = EditorProjectBuildState::from_catalog(&catalog);
    let controller = EditorProjectBuildController {
        daemon_endpoint: session.daemon_endpoint.clone(),
        project_id: session.project_id,
    };

    if !controller_set::complete_project_build(cx, fence, controller) {
        return;
    }
    cx.set_global(catalog);
    cx.set_global(state);
    info!(
        profile_count,
        output_target_count, diagnostic_count, "installed project build controller"
    );
}

pub fn install_project_build_action_handlers(cx: &mut App) {
    cx.on_action(|action: &SetProjectBuildProfile, cx| {
        let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>().cloned() else {
            warn!("ignored project build profile selection without a build catalog");
            return;
        };
        let selected = cx
            .default_global::<EditorProjectBuildState>()
            .select_profile(&action.profile, &catalog);
        info!(
            profile = %action.profile,
            selected,
            "project build profile selected"
        );
        cx.refresh_windows();
    });

    cx.on_action(|action: &SetProjectBuildTarget, cx| {
        let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>().cloned() else {
            warn!("ignored project build target selection without a build catalog");
            return;
        };
        let selected = cx
            .default_global::<EditorProjectBuildState>()
            .select_target(&action.target_key, &catalog);
        info!(
            target_key = %action.target_key,
            selected,
            "project build cargo target selected"
        );
        cx.refresh_windows();
    });

    cx.on_action(|_: &PlanProjectBuild, cx| {
        plan_project_build_from_action(cx);
    });

    cx.on_action(|_: &ExecuteProjectBuild, cx| {
        execute_project_build_from_action(cx);
    });
}

pub fn plan_project_build_from_action(cx: &mut App) {
    let attached = match controller_set::project_build_controller(cx) {
        Ok(attached) => attached,
        Err(error) => {
            publish_project_build_failure(cx, error.to_string());
            return;
        }
    };
    let controller = attached.controller;
    let fence = attached.fence;
    let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>().cloned() else {
        publish_project_build_failure(cx, "project build catalog is not loaded");
        return;
    };
    let selection = {
        let state = cx.default_global::<EditorProjectBuildState>();
        let Some(profile) = state.selected_profile(&catalog).cloned() else {
            publish_project_build_failure(cx, "project manifest has no build profiles");
            return;
        };
        let Some(target) = state.selected_target(&catalog).cloned() else {
            publish_project_build_failure(cx, "project build catalog has no cargo target options");
            return;
        };
        (profile, target)
    };

    let (profile, target) = selection;
    {
        let state = cx.default_global::<EditorProjectBuildState>();
        state.select_profile(&profile.name, &catalog);
        state.select_target(&target.key, &catalog);
        state.mark_planning();
    }
    publish_project_build_console_log(
        cx,
        LogLevel::Info,
        format!(
            "Planning project build profile `{}` for cargo target `{}`",
            profile.name, target.label
        ),
    );
    cx.refresh_windows();

    let profile_name = profile.name;
    let target_key = target.key.clone();
    let target_label = target.label.clone();
    let target_triple = target.target_triple;
    info!(
        project_id = %controller.project_id,
        profile = %profile_name,
        target = %target_label,
        "planning project build"
    );

    let project_id = controller.project_id.clone();
    let daemon_endpoint = controller.daemon_endpoint;
    let request_profile = profile_name.clone();
    crate::rpc_runtime::spawn_editor_rpc(
        cx,
        "project-build-plan",
        move || async move {
            let daemon = AzDaemonClient::connect(&daemon_endpoint).await?;
            daemon
                .plan_project_build(&project_id, &request_profile, target_triple.as_deref())
                .await
        },
        move |cx, result| match result {
            Ok(plan) => {
                if controller_set::is_current_fence(cx, fence) {
                    publish_project_build_plan(cx, profile_name, target_key, target_label, plan);
                }
            }
            Err(err) => {
                if !controller_set::is_current_fence(cx, fence) {
                    return;
                }
                error!(error = %err, "failed to plan project build");
                let message = err.to_string();
                publish_project_build_failure(cx, message);
            }
        },
    );
}

/// Admission for one execute request: resolves the attached controller, the
/// build catalog, and the selected profile/target, publishing the failure and
/// returning `None` when the build cannot start.
fn resolve_project_build_execution(
    cx: &mut App,
) -> Option<(
    EditorProjectBuildController,
    ControllerFence,
    EditorProjectBuildCatalog,
    EditorBuildProfileData,
    EditorBuildTargetData,
)> {
    let attached = match controller_set::project_build_controller(cx) {
        Ok(attached) => attached,
        Err(error) => {
            publish_project_build_failure(cx, error.to_string());
            return None;
        }
    };
    let Some(catalog) = cx.try_global::<EditorProjectBuildCatalog>().cloned() else {
        publish_project_build_failure(cx, "project build catalog is not loaded");
        return None;
    };
    let state = cx.default_global::<EditorProjectBuildState>();
    if !state.can_execute(&catalog) {
        if state.phase.is_busy() {
            publish_project_build_console_log(
                cx,
                LogLevel::Warn,
                "Build is already running; ignoring duplicate build request",
            );
        } else {
            publish_project_build_failure(cx, "project build profile or target is unavailable");
        }
        cx.refresh_windows();
        return None;
    }
    let Some(profile) = state.selected_profile(&catalog).cloned() else {
        publish_project_build_failure(cx, "project manifest has no build profiles");
        return None;
    };
    let Some(target) = state.selected_target(&catalog).cloned() else {
        publish_project_build_failure(cx, "project build catalog has no cargo target options");
        return None;
    };
    Some((
        attached.controller,
        attached.fence,
        catalog,
        profile,
        target,
    ))
}

/// Move the build state into `Running` and announce the start on the console.
fn begin_project_build_execution(
    cx: &mut App,
    catalog: &EditorProjectBuildCatalog,
    profile: &EditorBuildProfileData,
    target: &EditorBuildTargetData,
) {
    {
        let state = cx.default_global::<EditorProjectBuildState>();
        state.select_profile(&profile.name, catalog);
        state.select_target(&target.key, catalog);
        state.mark_running();
    }
    publish_project_build_console_log(
        cx,
        LogLevel::Info,
        format!(
            "Executing project build profile `{}` for cargo target `{}`",
            profile.name, target.label
        ),
    );
    cx.refresh_windows();
}

pub fn execute_project_build_from_action(cx: &mut App) {
    let Some((controller, fence, catalog, profile, target)) = resolve_project_build_execution(cx)
    else {
        return;
    };
    begin_project_build_execution(cx, &catalog, &profile, &target);

    let profile_name = profile.name;
    let target_key = target.key.clone();
    let target_label = target.label.clone();
    let target_triple = target.target_triple;
    let project_id = controller.project_id.clone();
    let daemon_endpoint = controller.daemon_endpoint;
    let background = cx.background_executor().clone();
    let (progress_tx, mut progress_rx) = unbounded_channel::<ProjectBuildProgressUpdate>();
    info!(
        project_id = %project_id,
        profile = %profile_name,
        target = %target_label,
        "executing project build"
    );

    cx.spawn(async move |cx| {
        let build_project_id = project_id.clone();
        let build_profile = profile_name.clone();
        let mut build = Box::pin(background.spawn(async move {
            crate::rpc_runtime::block_on_editor_rpc(async move {
                let daemon = AzDaemonClient::connect(&daemon_endpoint).await?;
                let sink = EditorBuildProgressSink::new(progress_tx).into_client();
                daemon
                    .execute_project_build(
                        &build_project_id,
                        &build_profile,
                        target_triple.as_deref(),
                        sink,
                    )
                    .await
            })
        }));

        let mut last_seq = 0_u64;
        let result = loop {
            tokio::select! {
                update = progress_rx.recv() => {
                    let Some(update) = update else {
                        break build.await;
                    };
                    publish_project_build_progress_update(cx, fence, &mut last_seq, &update);
                }
                result = &mut build => {
                    while let Ok(update) = progress_rx.try_recv() {
                        publish_project_build_progress_update(cx, fence, &mut last_seq, &update);
                    }
                    break result;
                }
            }
        };

        match result {
            Ok(result) => {
                let success = result.success;
                info!(
                    project_id = %project_id,
                    profile = %profile_name,
                    target = %target_label,
                    success,
                    "project build execution finished"
                );
                cx.update(move |cx| {
                    if controller_set::is_current_fence(cx, fence) {
                        publish_project_build_execution_result(
                            cx,
                            profile_name,
                            target_key,
                            target_label,
                            result,
                        );
                    }
                });
            }
            Err(err) => {
                error!(error = %err, "failed to execute project build");
                let message = err.to_string();
                cx.update(move |cx| {
                    if controller_set::is_current_fence(cx, fence) {
                        publish_project_build_failure(cx, message);
                    }
                });
            }
        }
    })
    .detach();
}

fn publish_project_build_progress_update(
    cx: &gpui::AsyncApp,
    fence: ControllerFence,
    last_seq: &mut u64,
    update: &ProjectBuildProgressUpdate,
) {
    if update.seq <= *last_seq {
        return;
    }
    *last_seq = update.seq;
    let ui_progress = update.to_ui_data();
    let message = ui_progress.message.clone();
    let () = cx.update(move |cx| {
        if !controller_set::is_current_fence(cx, fence) {
            return;
        }
        let changed = cx
            .default_global::<EditorProjectBuildState>()
            .mark_progress(ui_progress);
        if changed && !message.trim().is_empty() {
            publish_project_build_console_log(
                cx,
                build_progress_log_level(&message),
                message.clone(),
            );
        }
        cx.refresh_windows();
    });
}

fn publish_project_build_execution_result(
    cx: &mut App,
    profile: String,
    target_key: String,
    target_label: String,
    result: ProjectBuildExecutionResult,
) {
    let ui_plan = project_build_plan_to_ui(result.plan, profile, target_key, target_label);
    let command_count = ui_plan.command_count;
    let completed = result.completed_command_count;
    if result.success {
        cx.default_global::<EditorProjectBuildState>()
            .mark_succeeded(ui_plan);
        publish_project_build_console_log(
            cx,
            LogLevel::Info,
            format!("Build completed: {completed}/{command_count} command(s) finished"),
        );
        cx.refresh_windows();
        return;
    }

    let headline = if result.diagnostic_headline.trim().is_empty() {
        "project build failed".to_owned()
    } else {
        result.diagnostic_headline
    };
    {
        let state = cx.default_global::<EditorProjectBuildState>();
        state.last_plan = Some(ui_plan);
        state.mark_failed(headline.clone());
    }
    publish_project_build_console_log(
        cx,
        LogLevel::Error,
        format!("Build failed after {completed}/{command_count} command(s): {headline}"),
    );
    if !result.diagnostic_tail.trim().is_empty() {
        publish_project_build_output_line(cx, LogLevel::Error, result.diagnostic_tail);
    }
    cx.refresh_windows();
}

fn publish_project_build_plan(
    cx: &mut App,
    profile: String,
    target_key: String,
    target_label: String,
    plan: ProjectBuildPlan,
) {
    let ui_plan = project_build_plan_to_ui(plan, profile, target_key, target_label);
    let command_count = ui_plan.command_count;
    let profile = ui_plan.profile.clone();
    let target_label = ui_plan.target_label.clone();
    let output_lines = project_build_plan_output_lines(&ui_plan);

    for (level, line) in output_lines {
        publish_project_build_console_log(cx, level, line);
    }
    cx.default_global::<EditorProjectBuildState>()
        .mark_planned(ui_plan);
    cx.refresh_windows();
    info!(
        profile = %profile,
        target = %target_label,
        command_count,
        "planned project build"
    );
}

fn publish_project_build_failure(cx: &mut App, diagnostic: impl Into<String>) {
    let diagnostic = diagnostic.into();
    cx.default_global::<EditorProjectBuildState>()
        .mark_failed(diagnostic.clone());
    publish_project_build_console_log(cx, LogLevel::Error, diagnostic);
    cx.refresh_windows();
}

fn publish_project_build_console_log(cx: &mut App, level: LogLevel, message: impl Into<String>) {
    let message = message.into();
    cx.default_global::<ConsoleState>()
        .log_from_source(level, BUILD_LOG_SOURCE, message);
}

fn publish_project_build_output_line(cx: &mut App, level: LogLevel, message: impl Into<String>) {
    let message = message.into();
    cx.default_global::<OutputLogState>()
        .append_output(level, BUILD_LOG_SOURCE, message.clone());
    cx.default_global::<ConsoleState>()
        .log_from_source(level, BUILD_LOG_SOURCE, message);
}

fn build_progress_log_level(message: &str) -> LogLevel {
    let message = message.trim().to_ascii_lowercase();
    if message.starts_with("failed") || message.starts_with("error") || message.contains("error[") {
        LogLevel::Error
    } else if message.starts_with("warning") {
        LogLevel::Warn
    } else {
        LogLevel::Info
    }
}

pub(crate) fn load_project_build_catalog(project_root: &Path) -> EditorProjectBuildCatalog {
    let lock_path = project_root.join("azoth.lock");
    let manifest_path = project_root.join("azoth.toml");
    let mut diagnostics = Vec::new();
    let lock = read_project_toml(&lock_path, &mut diagnostics);
    let manifest = read_project_toml(&manifest_path, &mut diagnostics);

    if lock.is_none() && manifest.is_none() {
        diagnostics.push(format!(
            "no azoth.lock or azoth.toml found under `{}`",
            project_root.display()
        ));
    }

    let profiles = lock
        .as_ref()
        .and_then(build_profiles_from_toml)
        .filter(|profiles| !profiles.is_empty())
        .or_else(|| {
            manifest
                .as_ref()
                .and_then(build_profiles_from_toml)
                .filter(|profiles| !profiles.is_empty())
        })
        .unwrap_or_default();
    let default_build_targets = lock
        .as_ref()
        .and_then(default_build_targets_from_toml)
        .filter(|targets| !targets.is_empty())
        .or_else(|| {
            manifest
                .as_ref()
                .and_then(default_build_targets_from_toml)
                .filter(|targets| !targets.is_empty())
        })
        .unwrap_or_default();

    if profiles.is_empty() {
        diagnostics.push("project manifest declares no build profiles".to_owned());
    }
    if default_build_targets.is_empty() {
        diagnostics.push("project manifest declares no default build targets".to_owned());
    }

    EditorProjectBuildCatalog {
        profiles,
        targets: vec![EditorBuildTargetData::host()],
        default_build_targets,
        diagnostics,
    }
}

fn read_project_toml(path: &Path, diagnostics: &mut Vec<String>) -> Option<Value> {
    if !path.exists() {
        return None;
    }
    match fs::read_to_string(path) {
        Ok(contents) => match toml::from_str::<Value>(&contents) {
            Ok(value) => Some(value),
            Err(error) => {
                diagnostics.push(format!("failed to parse `{}`: {error}", path.display()));
                None
            }
        },
        Err(error) => {
            diagnostics.push(format!("failed to read `{}`: {error}", path.display()));
            None
        }
    }
}

fn build_profiles_from_toml(value: &Value) -> Option<Vec<EditorBuildProfileData>> {
    profile_array(value).map(|profiles| {
        profiles
            .iter()
            .filter_map(Value::as_table)
            .filter_map(|profile| {
                let name = string_value(Some(profile.get("name")?));
                if name.trim().is_empty() {
                    return None;
                }
                Some(EditorBuildProfileData {
                    name,
                    asset_platform: string_value(profile.get("asset_platform")),
                    cargo_profile: string_value(profile.get("cargo_profile")),
                    container: string_value(profile.get("container")),
                    compression: string_value(profile.get("compression")),
                })
            })
            .collect()
    })
}

fn profile_array(value: &Value) -> Option<&Vec<Value>> {
    value
        .get("build")
        .and_then(|build| build.get("profiles"))
        .and_then(Value::as_array)
        .or_else(|| {
            value
                .get("packaging")
                .and_then(|packaging| packaging.get("profiles"))
                .and_then(Value::as_array)
        })
}

fn default_build_targets_from_toml(value: &Value) -> Option<Vec<EditorBuildOutputTargetData>> {
    value
        .get("tools")
        .and_then(|tools| tools.get("build_targets"))
        .and_then(Value::as_array)
        .map(|targets| {
            targets
                .iter()
                .filter_map(Value::as_table)
                .filter(|target| {
                    target
                        .get("default")
                        .and_then(Value::as_bool)
                        .unwrap_or(true)
                })
                .filter_map(|target| {
                    let name = string_value(target.get("name"));
                    if name.trim().is_empty() {
                        return None;
                    }
                    Some(EditorBuildOutputTargetData {
                        owner_id: string_value(target.get("owner_id")),
                        name,
                        kind: string_value(target.get("kind")),
                        role: string_value(target.get("role")),
                        default: target
                            .get("default")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                    })
                })
                .collect()
        })
}

fn string_value(value: Option<&Value>) -> String {
    value.and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn project_build_plan_to_ui(
    plan: ProjectBuildPlan,
    profile: String,
    target_key: String,
    target_label: String,
) -> EditorProjectBuildPlanData {
    let package_profile = plan
        .package_profile
        .map(project_build_package_profile_to_ui);
    let commands = plan
        .commands
        .into_iter()
        .map(|command| EditorProjectBuildCommandData {
            owner_id: command.owner_id,
            owner_root: command.owner_root,
            target_name: command.target_name,
            program: command.program,
            cwd: command.cwd,
            args: command.args,
        })
        .collect::<Vec<_>>();
    EditorProjectBuildPlanData {
        profile,
        target_key,
        target_label,
        command_count: commands.len(),
        commands,
        package_profile,
    }
}

fn project_build_package_profile_to_ui(
    profile: ProjectBuildPackageProfile,
) -> EditorBuildProfileData {
    EditorBuildProfileData {
        name: profile.name,
        asset_platform: profile.asset_platform,
        cargo_profile: profile.cargo_profile,
        container: profile.container,
        compression: profile.compression,
    }
}

fn project_build_plan_output_lines(plan: &EditorProjectBuildPlanData) -> Vec<(LogLevel, String)> {
    let mut lines = Vec::with_capacity(plan.commands.len() + 3);
    lines.push((
        LogLevel::Info,
        format!(
            "Build plan `{}` for target `{}` contains {} command(s)",
            plan.profile, plan.target_label, plan.command_count
        ),
    ));
    if let Some(profile) = &plan.package_profile {
        lines.push((
            LogLevel::Info,
            format!(
                "Package profile `{}`: assets={} cargo={} container={} compression={}",
                profile.name,
                empty_as_unset(&profile.asset_platform),
                empty_as_unset(&profile.cargo_profile),
                empty_as_unset(&profile.container),
                empty_as_unset(&profile.compression)
            ),
        ));
    }
    for command in &plan.commands {
        lines.push((
            LogLevel::Info,
            format!(
                "[{}:{}] {}> {}",
                empty_as_unset(&command.owner_id),
                command.target_name,
                command.cwd,
                shell_command_line(&command.program, &command.args)
            ),
        ));
    }
    lines
}

fn empty_as_unset(value: &str) -> &str {
    if value.trim().is_empty() {
        "unset"
    } else {
        value
    }
}

fn shell_command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .map(shell_quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/' | '\\' | ':'))
    {
        value.to_owned()
    } else {
        format!("\"{}\"", value.replace('"', "\\\""))
    }
}

pub fn build_status_line_from_state(
    state: &EditorProjectBuildState,
    catalog: &EditorProjectBuildCatalog,
) -> Option<(String, String, EditorProjectBuildPhase)> {
    let profile = state
        .selected_profile(catalog)
        .map_or("no profile", |profile| profile.name.as_str());
    let target = state
        .selected_target(catalog)
        .map_or("no target", |target| target.label.as_str());

    let line = match state.phase {
        EditorProjectBuildPhase::Idle => (
            "Build".to_owned(),
            format!("{profile} · {target} · idle"),
            state.phase,
        ),
        EditorProjectBuildPhase::Planning => (
            "Build".to_owned(),
            format!("planning {profile} · {target}"),
            state.phase,
        ),
        EditorProjectBuildPhase::Running => {
            let progress = state.progress.as_ref();
            let label =
                progress.map_or_else(|| "Build starting".to_owned(), project_build_running_label);
            let detail = progress.map_or_else(
                || format!("starting {profile} · {target}"),
                |progress| {
                    let command_number = progress.command_index.saturating_add(1);
                    let mut detail = format!(
                        "command {}/{} · {}",
                        command_number, progress.command_count, progress.target_name
                    );
                    if !progress.message.trim().is_empty() {
                        detail.push_str(" · ");
                        detail.push_str(&progress.message);
                    }
                    if let Some(total) = progress.command_total.filter(|total| *total > 0) {
                        let _ = write!(
                            detail,
                            " · {}/{} units",
                            progress.command_done.min(total),
                            total
                        );
                    } else if progress.command_done > 0 {
                        let _ = write!(detail, " · {} units", progress.command_done);
                    }
                    detail
                },
            );
            (label, detail, state.phase)
        }
        EditorProjectBuildPhase::Planned => {
            let command_count = state
                .last_plan
                .as_ref()
                .map_or(0, |plan| plan.command_count);
            (
                "Build plan".to_owned(),
                format!("{command_count} command(s) planned"),
                state.phase,
            )
        }
        EditorProjectBuildPhase::Succeeded => {
            let command_count = state
                .last_plan
                .as_ref()
                .map_or(0, |plan| plan.command_count);
            (
                "Build complete".to_owned(),
                format!("{command_count} command(s) finished · {profile} · {target}"),
                state.phase,
            )
        }
        EditorProjectBuildPhase::Failed => (
            "Build failed".to_owned(),
            state
                .diagnostic
                .clone()
                .unwrap_or_else(|| "planning failed".to_owned()),
            state.phase,
        ),
    };
    Some(line)
}

fn project_build_running_label(progress: &EditorProjectBuildProgressData) -> String {
    progress
        .command_total
        .filter(|total| *total > 0)
        .map_or_else(
            || {
                if progress.command_done > 0 {
                    "Building live".to_owned()
                } else {
                    "Building".to_owned()
                }
            },
            |total| {
                format!(
                    "Building {}",
                    command_percent_label(progress.command_done, total)
                )
            },
        )
}

fn command_percent_label(done: u64, total: u64) -> String {
    if done >= total {
        "100%".to_owned()
    } else {
        let percent = u128::from(done) * 100 / u128::from(total);
        format!("{percent}%")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_editor_ui::HOST_TARGET_KEY;

    #[test]
    fn load_project_build_catalog_projects_profiles_and_default_targets() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("azoth.lock"),
            r#"
[[packaging.profiles]]
name = "pc-dev"
asset_platform = "pc"
cargo_profile = "debug"
container = "loose"
compression = "none"

[[packaging.profiles]]
name = "pc-release"
asset_platform = "pc"
cargo_profile = "release"
container = "azpack"
compression = "oodle"

[[tools.build_targets]]
owner_id = "local.test"
name = "game"
role = "project-services"
kind = "package"
default = true

[[tools.build_targets]]
owner_id = "local.test"
name = "tool"
role = "tool"
kind = "package"
default = false
"#,
        )
        .expect("write lockfile");

        let catalog = load_project_build_catalog(temp.path());

        assert_eq!(
            catalog
                .profiles
                .iter()
                .map(|profile| profile.name.as_str())
                .collect::<Vec<_>>(),
            vec!["pc-dev", "pc-release"]
        );
        assert_eq!(catalog.targets.len(), 1);
        assert_eq!(catalog.targets[0].key, HOST_TARGET_KEY);
        assert_eq!(catalog.default_build_targets.len(), 1);
        assert_eq!(catalog.default_build_targets[0].name, "game");
        assert!(catalog.diagnostics.is_empty());
    }

    #[test]
    fn load_project_build_catalog_falls_back_to_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::write(
            temp.path().join("azoth.toml"),
            r#"
[[build.profiles]]
name = "editor-dev"
asset_platform = "pc"
cargo_profile = "debug"
container = "loose"
compression = "none"

[[tools.build_targets]]
name = "editor"
role = "tool"
kind = "package"
"#,
        )
        .expect("write manifest");

        let catalog = load_project_build_catalog(temp.path());

        assert_eq!(catalog.profiles[0].name, "editor-dev");
        assert_eq!(catalog.default_build_targets[0].name, "editor");
        assert!(catalog.diagnostics.is_empty());
    }

    #[test]
    fn build_status_line_reports_running_command_percent_and_command_phase() {
        let catalog = EditorProjectBuildCatalog {
            profiles: vec![EditorBuildProfileData {
                name: "pc-dev".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "debug".to_string(),
                container: "loose".to_string(),
                compression: "none".to_string(),
            }],
            targets: vec![EditorBuildTargetData::host()],
            default_build_targets: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut state = EditorProjectBuildState::from_catalog(&catalog);
        state.mark_progress(EditorProjectBuildProgressData {
            seq: 1,
            command_index: 0,
            command_count: 2,
            target_name: "game".to_string(),
            done_bp: 2_500,
            command_done: 5,
            command_total: Some(10),
            message: "Starting command 1/2: local.example:game".to_string(),
        });

        let (label, sub, phase) = build_status_line_from_state(&state, &catalog).unwrap();

        assert_eq!(phase, EditorProjectBuildPhase::Running);
        assert_eq!(label, "Building 50%");
        assert!(sub.contains("command 1/2"));
        assert!(sub.contains("game"));
        assert!(sub.contains("5/10 units"));
    }

    #[test]
    fn build_status_line_does_not_report_aggregate_percent_for_unknown_command_total() {
        let catalog = EditorProjectBuildCatalog {
            profiles: vec![EditorBuildProfileData {
                name: "pc-dev".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "debug".to_string(),
                container: "loose".to_string(),
                compression: "none".to_string(),
            }],
            targets: vec![EditorBuildTargetData::host()],
            default_build_targets: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut state = EditorProjectBuildState::from_catalog(&catalog);
        state.mark_progress(EditorProjectBuildProgressData {
            seq: 1,
            command_index: 0,
            command_count: 2,
            target_name: "game".to_string(),
            done_bp: 2_500,
            command_done: 5,
            command_total: None,
            message: "compiling game".to_string(),
        });

        let (label, sub, phase) = build_status_line_from_state(&state, &catalog).unwrap();

        assert_eq!(phase, EditorProjectBuildPhase::Running);
        assert_eq!(label, "Building live");
        assert!(sub.contains("command 1/2"));
        assert!(sub.contains("5 units"));
        assert!(!label.contains("25%"));
    }

    #[test]
    fn build_status_line_reports_successful_execution() {
        let catalog = EditorProjectBuildCatalog {
            profiles: vec![EditorBuildProfileData {
                name: "pc-dev".to_string(),
                asset_platform: "pc".to_string(),
                cargo_profile: "debug".to_string(),
                container: "loose".to_string(),
                compression: "none".to_string(),
            }],
            targets: vec![EditorBuildTargetData::host()],
            default_build_targets: Vec::new(),
            diagnostics: Vec::new(),
        };
        let mut state = EditorProjectBuildState::from_catalog(&catalog);
        state.mark_succeeded(EditorProjectBuildPlanData {
            profile: "pc-dev".to_string(),
            target_key: HOST_TARGET_KEY.to_string(),
            target_label: "Host".to_string(),
            command_count: 2,
            commands: Vec::new(),
            package_profile: None,
        });

        let (label, sub, phase) = build_status_line_from_state(&state, &catalog).unwrap();

        assert_eq!(phase, EditorProjectBuildPhase::Succeeded);
        assert_eq!(label, "Build complete");
        assert!(sub.contains("2 command(s) finished"));
    }
}
