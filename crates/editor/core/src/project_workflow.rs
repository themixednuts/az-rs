//! Editor-owned project workflow controller.
//!
//! This module lets the standalone editor drive the same universal
//! project-scaffold workflows as the CLI without linking project/game code into
//! the editor process.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use az_editor_ui::panels::project_workflow as ui;
use az_editor_ui::panels::{
    ConsoleState, EditorAddableAuthoredComponents, EditorCreatableAuthoredSchemas,
    EditorProjectConnectionState, EditorTypeRegistry, LogLevel,
};
use az_proto_asset::{ASSET_PROCESSOR_SERVICE_NAME, ASSET_WORKER_SERVICE_NAME};
use az_proto_core::{Endpoint, EndpointKind};
use az_proto_project::PROJECT_HOST_SERVICE_NAME;
use gpui::{App, Global};
use tokio::runtime::Builder;
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::task::LocalSet;
use tracing::{error, info, instrument};

use crate::EditorAttachSession;
use crate::app::{
    OpenAssetProcessorAfterAttach, gem_selection_from_project_inventory,
    open_or_focus_asset_processor_window,
};
use crate::attach_to_session_via_daemon;
use crate::authored_selection::addable_reflected_component_data;
use crate::controller_set::install_attached_controllers;
use crate::daemon::AzDaemonClient;
use crate::daemon_bootstrap::ensure_daemon_endpoint_for_project;
use crate::error::{EditorError, EditorResult};
use crate::game_data_catalog::EditorGameDataCatalog;
use crate::project_open_progress::{self, OpenProgressUpdate};

const EDITOR_SESSION_SERVICE_START_TIMEOUT_MS: u64 = 60 * 60 * 1_000;

fn editor_required_session_service_names() -> Vec<String> {
    [
        PROJECT_HOST_SERVICE_NAME.to_string(),
        ASSET_PROCESSOR_SERVICE_NAME.to_string(),
        ASSET_WORKER_SERVICE_NAME.to_string(),
    ]
    .into()
}

#[derive(Clone, Debug, Default)]
pub struct EditorProjectWorkflowController;

impl Global for EditorProjectWorkflowController {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowCreateRequest {
    pub name: String,
    pub path: PathBuf,
    pub lore_url: Option<String>,
    pub topology: az_project_scaffold::ProjectTopologyKind,
    pub enabled_gems: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowInitRequest {
    pub path: PathBuf,
    pub name: Option<String>,
    pub lore_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowOutcome {
    pub operation: ui::Operation,
    pub project_root: PathBuf,
    pub message: String,
    pub next_steps: Vec<ui::NextStep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowSessionRequest {
    pub project_root: PathBuf,
    pub session_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowSessionOutcome {
    pub project_root: PathBuf,
    pub project_id: String,
    pub session_slug: String,
    pub workspace_root: String,
    pub created: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowServicePrepareRequest {
    pub project_root: PathBuf,
    pub session_slug: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowServicePrepareOutcome {
    pub project_root: PathBuf,
    pub project_id: String,
    pub session_slug: String,
    pub service_names: Vec<String>,
    pub built: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowEditorSessionOpenRequest {
    pub project_root: PathBuf,
    pub session_name: String,
}

#[derive(Debug, Clone)]
pub struct ProjectWorkflowEditorSessionOpenOutcome {
    pub project_root: PathBuf,
    pub project_id: String,
    pub session_slug: String,
    pub running_service_names: Vec<String>,
    pub attach_session: EditorAttachSession,
}

pub fn install_project_workflow_action_handlers(cx: &mut App) {
    install_project_launcher_action_handlers(cx);
    install_project_scaffold_action_handlers(cx);
    install_project_session_action_handlers(cx);
}

/// Launcher-mode transitions: they only publish workflow status and never run
/// a scaffold or daemon workflow.
fn install_project_launcher_action_handlers(cx: &mut App) {
    cx.on_action(|_: &az_editor_ui::actions::NewProject, cx| {
        publish_project_workflow_status(cx, ui::Status::new_project_requested());
    });
    cx.on_action(|_: &az_editor_ui::actions::OpenProject, cx| {
        publish_project_workflow_status(cx, ui::Status::existing_project_requested());
    });
    cx.on_action(|_: &az_editor_ui::actions::BackToProjectLauncher, cx| {
        // A connecting/failed open returns to the launcher: drop the connecting
        // gate so the shell renders the project launcher again. The Recent entry
        // recorded at select-time stays.
        info!("project workflow returning to launcher from connecting/failed open");
        cx.remove_global::<EditorProjectConnectionState>();
        publish_project_workflow_status(cx, ui::Status::existing_project_requested());
    });
}

/// In-process scaffold workflows: create a project, create a gem, or
/// initialize an existing directory.
fn install_project_scaffold_action_handlers(cx: &mut App) {
    cx.on_action(|action: &az_editor_ui::actions::CreateProject, cx| {
        let request = match create_request_from_action(action) {
            Ok(request) => request,
            Err(err) => {
                publish_project_workflow_error(cx, ui::Operation::CreateProject, "", &err);
                return;
            }
        };
        let project_root = request.project_root_label();
        publish_project_workflow_status(
            cx,
            ui::Status::running(ui::Operation::CreateProject, project_root.clone()),
        );

        match create_project(&request) {
            Ok(outcome) => publish_project_workflow_outcome(cx, outcome),
            Err(err) => {
                publish_project_workflow_error(
                    cx,
                    ui::Operation::CreateProject,
                    project_root,
                    &err,
                );
            }
        }
    });
    cx.on_action(|action: &az_editor_ui::actions::CreateGem, cx| {
        let request = match create_gem_request_from_action(action, cx) {
            Ok(request) => request,
            Err(err) => {
                publish_gem_creation_error(cx, &err);
                return;
            }
        };
        match create_gem(&request) {
            Ok(message) => publish_gem_creation_success(cx, message),
            Err(err) => publish_gem_creation_error(cx, &err),
        }
    });
    cx.on_action(
        |action: &az_editor_ui::actions::InitializeProjectWorkflow, cx| {
            let request = match init_request_from_action(action) {
                Ok(request) => request,
                Err(err) => {
                    publish_project_workflow_error(cx, ui::Operation::InitializeProject, "", &err);
                    return;
                }
            };
            let project_root = request.project_root_label();
            publish_project_workflow_status(
                cx,
                ui::Status::running(ui::Operation::InitializeProject, project_root.clone()),
            );

            match initialize_project(&request) {
                Ok(outcome) => publish_project_workflow_outcome(cx, outcome),
                Err(err) => publish_project_workflow_error(
                    cx,
                    ui::Operation::InitializeProject,
                    project_root,
                    &err,
                ),
            }
        },
    );
}

/// Daemon-backed session workflows: ensure a session, prepare its services,
/// and open the editor session the shell attaches to.
fn install_project_session_action_handlers(cx: &mut App) {
    install_ensure_project_session_action_handler(cx);
    install_prepare_project_services_action_handler(cx);
    install_open_editor_session_action_handler(cx);
}

/// Create-or-adopt the named session through azd, reporting the outcome on the
/// workflow status line.
fn install_ensure_project_session_action_handler(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::EnsureProjectWorkflowSession, cx| {
            let request = match ensure_session_request_from_action(action) {
                Ok(request) => request,
                Err(err) => {
                    publish_project_workflow_error(
                        cx,
                        ui::Operation::EnsureProjectSession,
                        "",
                        &err,
                    );
                    return;
                }
            };
            let project_root = path_label(&request.project_root);
            let remaining_steps = current_project_workflow_steps_after_session_bootstrap(cx);
            publish_project_workflow_status(
                cx,
                ui::Status::running(ui::Operation::EnsureProjectSession, project_root.clone()),
            );

            match ensure_project_workflow_session(request) {
                Ok(outcome) => {
                    let message = if outcome.created {
                        format!(
                            "Created session `{}` for project `{}`",
                            outcome.session_slug, outcome.project_id
                        )
                    } else {
                        format!(
                            "Session `{}` already exists for project `{}`",
                            outcome.session_slug, outcome.project_id
                        )
                    };
                    publish_console_log(cx, LogLevel::Info, "project-workflow", message.clone());
                    publish_project_workflow_status(
                        cx,
                        ui::Status::succeeded(
                            ui::Operation::EnsureProjectSession,
                            path_label(&outcome.project_root),
                            message,
                            remaining_steps,
                        ),
                    );
                }
                Err(err) => publish_project_workflow_error(
                    cx,
                    ui::Operation::EnsureProjectSession,
                    project_root,
                    &err,
                ),
            }
        },
    );
}

/// Start the session's project services through azd ahead of an editor attach.
fn install_prepare_project_services_action_handler(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::PrepareProjectWorkflowSessionServices, cx| {
            let request = match prepare_services_request_from_action(action) {
                Ok(request) => request,
                Err(err) => {
                    publish_project_workflow_error(
                        cx,
                        ui::Operation::PrepareProjectServices,
                        "",
                        &err,
                    );
                    return;
                }
            };
            let project_root = path_label(&request.project_root);
            let remaining_steps = current_project_workflow_steps_after_service_preparation(cx);
            publish_project_workflow_status(
                cx,
                ui::Status::running(ui::Operation::PrepareProjectServices, project_root.clone()),
            );

            match prepare_project_workflow_session_services(request) {
                Ok(outcome) => {
                    let message = format!(
                        "Prepared {} services for session `{}`",
                        outcome.service_names.len(),
                        outcome.session_slug
                    );
                    publish_console_log(cx, LogLevel::Info, "project-workflow", message.clone());
                    publish_project_workflow_status(
                        cx,
                        ui::Status::succeeded(
                            ui::Operation::PrepareProjectServices,
                            path_label(&outcome.project_root),
                            message,
                            remaining_steps,
                        ),
                    );
                }
                Err(err) => publish_project_workflow_error(
                    cx,
                    ui::Operation::PrepareProjectServices,
                    project_root,
                    &err,
                ),
            }
        },
    );
}

/// Open the editor session the shell attaches to. This is the only handler
/// that transitions the shell into the loaded-project workspace.
fn install_open_editor_session_action_handler(cx: &mut App) {
    cx.on_action(
        |action: &az_editor_ui::actions::OpenProjectWorkflowEditorSession, cx| {
            open_project_workflow_editor_session_from_action(action, cx);
        },
    );
}

pub(crate) fn open_project_workflow_editor_session_from_action(
    action: &az_editor_ui::actions::OpenProjectWorkflowEditorSession,
    cx: &mut App,
) -> bool {
    let status = cx.try_global::<ui::Status>().cloned();
    let attached = cx.try_global::<EditorAttachSession>().is_some();
    info!(
        project_root = %action.project_root,
        session_name = %action.session_name,
        attached,
        status_phase = ?status.as_ref().map(|status| status.phase),
        status_operation = ?status.as_ref().and_then(|status| status.operation),
        "project workflow open action received"
    );
    if attached {
        info!(
            project_root = %action.project_root,
            session_name = %action.session_name,
            "project workflow open action ignored because editor is already attached"
        );
        return false;
    }
    if project_workflow_operation_running(cx, ui::Operation::OpenEditorSession) {
        info!(
            project_root = %action.project_root,
            session_name = %action.session_name,
            "project workflow open action ignored because open is already running"
        );
        return true;
    }

    let request = match open_editor_session_request_from_action(action) {
        Ok(request) => request,
        Err(err) => {
            publish_project_workflow_error(cx, ui::Operation::OpenEditorSession, "", &err);
            return false;
        }
    };
    let project_root = path_label(&request.project_root);

    // Validate the picked folder is a real Azoth project BEFORE we transition
    // the shell. An invalid manifest surfaces inline and the open does not
    // proceed.
    match validate_project_root_for_open(&request.project_root) {
        Ok(manifest) => {
            // Record the project in Recent immediately — even a slow, failed,
            // or cancelled open leaves it discoverable.
            record_recent_project_from_manifest(&request.project_root, &manifest);
        }
        Err(err) => {
            publish_project_workflow_error(
                cx,
                ui::Operation::OpenEditorSession,
                project_root,
                &err,
            );
            return false;
        }
    }

    let remaining_steps = current_project_workflow_steps_after_service_preparation(cx);
    publish_project_workflow_status(
        cx,
        ui::Status::running(ui::Operation::OpenEditorSession, project_root.clone()),
    );
    // Switch the editor shell into the loaded-project workspace right away;
    // project-host-dependent panels gate on this connecting state until attach
    // completes.
    cx.set_global(EditorProjectConnectionState::connecting());

    spawn_open_project_workflow_editor_session(cx, request, project_root, remaining_steps);
    true
}

fn spawn_open_project_workflow_editor_session(
    cx: &App,
    request: ProjectWorkflowEditorSessionOpenRequest,
    project_root: String,
    remaining_steps: Vec<ui::NextStep>,
) {
    spawn_open_project_workflow_editor_session_with_daemon_endpoint(
        cx,
        request,
        None,
        project_root,
        remaining_steps,
    );
}

pub(crate) fn spawn_open_project_workflow_editor_session_with_daemon_endpoint(
    cx: &App,
    request: ProjectWorkflowEditorSessionOpenRequest,
    daemon_endpoint: Option<Endpoint>,
    project_root: String,
    remaining_steps: Vec<ui::NextStep>,
) {
    let background = cx.background_executor().clone();
    let session_name = request.session_name.clone();
    info!(
        project_root = %project_root,
        session_name = %session_name,
        "project workflow open background task spawning"
    );
    // Live progress channel: the background open task feeds decoded updates in,
    // and this foreground task drains them to publish a running-with-progress
    // status (the real bar) on the GPUI thread while the open proceeds.
    let (progress_tx, mut progress_rx) = unbounded_channel::<OpenProgressUpdate>();
    let progress_root = project_root.clone();
    cx.spawn(async move |cx| {
        info!(
            project_root = %project_root,
            session_name = %session_name,
            "project workflow open background task started"
        );
        let mut open = Box::pin(background.spawn(async move {
            if let Some(endpoint) = daemon_endpoint {
                open_project_workflow_editor_session_with_daemon_endpoint(
                    request,
                    endpoint,
                    Some(&progress_tx),
                )
            } else {
                open_project_workflow_editor_session_with_progress(request, &progress_tx)
            }
        }));

        // Progress capability lifetimes are owned by the RPC layer, so do not
        // require the channel to close before observing the open result. If the
        // daemon returns an error while the sink capability is still retained,
        // the UI must publish that error instead of staying on the last phase.
        let mut last_seq = 0u64;
        let result = loop {
            tokio::select! {
                update = progress_rx.recv() => {
                    let Some(update) = update else {
                        break open.await;
                    };
                    publish_open_project_progress_update(
                        cx,
                        &progress_root,
                        &mut last_seq,
                        &update,
                    );
                }
                result = &mut open => {
                    while let Ok(update) = progress_rx.try_recv() {
                        publish_open_project_progress_update(
                            cx,
                            &progress_root,
                            &mut last_seq,
                            &update,
                        );
                    }
                    break result;
                }
            }
        };

        match result {
            Ok(outcome) => {
                info!(
                    project_id = %outcome.project_id,
                    session_slug = %outcome.session_slug,
                    service_count = outcome.running_service_names.len(),
                    "project workflow open background task succeeded"
                );
                let message = format!(
                    "Opened session `{}` for project `{}` with {} running services",
                    outcome.session_slug,
                    outcome.project_id,
                    outcome.running_service_names.len()
                );
                cx.update(move |cx| {
                    install_opened_project_workflow_session(cx, outcome, message, remaining_steps);
                });
            }
            Err(err) => {
                error!(
                    error = %err,
                    project_root = %project_root,
                    session_name = %session_name,
                    "project workflow open background task failed"
                );
                cx.update(move |cx| {
                    // The shell switched into the loaded-project workspace and
                    // gated project-host panels on `Connecting` before this
                    // background task started (see
                    // `open_project_workflow_editor_session_from_action` /
                    // `app.rs` pending-open resume). Every failure path here —
                    // service start, attach validation, or the daemon RPC
                    // itself — funnels through this single `Err` arm, so this
                    // is the one place that must move the connection state out
                    // of `Connecting`. Without it, `EditorProjectConnectionState`
                    // never leaves `Connecting` and every gated panel spins
                    // forever with no visible error and no way back.
                    cx.set_global(EditorProjectConnectionState::failed(err.to_string()));
                    publish_project_workflow_error(
                        cx,
                        ui::Operation::OpenEditorSession,
                        project_root,
                        &err,
                    );
                    cx.refresh_windows();
                });
            }
        }
    })
    .detach();
}

/// Publish the successful open on the GPUI thread: install the verified
/// controller aggregate first, then let panels observe `Connected`.
fn install_opened_project_workflow_session(
    cx: &mut App,
    outcome: ProjectWorkflowEditorSessionOpenOutcome,
    message: String,
    remaining_steps: Vec<ui::NextStep>,
) {
    let attach_session = outcome.attach_session.clone();
    if let Err(error) = install_verified_attached_session(cx, attach_session) {
        let detail = error.to_string();
        error!(%error, "failed to install verified attached editor session");
        cx.set_global(EditorProjectConnectionState::failed(detail));
        publish_project_workflow_error(
            cx,
            ui::Operation::OpenEditorSession,
            path_label(&outcome.project_root),
            &error,
        );
        cx.refresh_windows();
        return;
    }

    // The validated aggregate is now synchronously visible in
    // `Installing` before any panel can observe Connected.
    cx.set_global(EditorProjectConnectionState::connected());
    publish_console_log(cx, LogLevel::Info, "project-workflow", message.clone());
    publish_project_workflow_status(
        cx,
        ui::Status::succeeded_with_attached_session(
            ui::Operation::OpenEditorSession,
            path_label(&outcome.project_root),
            message,
            remaining_steps,
            ui::AttachedSession::new(
                outcome.project_id,
                outcome.session_slug,
                outcome.running_service_names,
            ),
        ),
    );
    cx.refresh_windows();
}

fn publish_open_project_progress_update(
    cx: &gpui::AsyncApp,
    progress_root: &str,
    last_seq: &mut u64,
    update: &OpenProgressUpdate,
) {
    if update.seq <= *last_seq {
        return; // monotone: drop stale/out-of-order updates.
    }
    *last_seq = update.seq;
    let progress = update.to_progress_data();
    let project_root = progress_root.to_string();
    let () = cx.update(move |cx| {
        publish_project_workflow_status(
            cx,
            ui::Status::running_with_progress(
                ui::Operation::OpenEditorSession,
                project_root,
                progress,
            ),
        );
        cx.refresh_windows();
    });
}

/// Scaffold a new project through the shared project-scaffold workflow.
///
/// # Errors
///
/// Returns [`EditorError::ProjectWorkflow`] if
/// [`az_project_scaffold::new::execute_with_options`] fails — the target path
/// is not writable or already holds a project, the requested topology or
/// engine gems cannot be resolved, or the Lore repository URL is rejected.
pub fn create_project(
    request: &ProjectWorkflowCreateRequest,
) -> EditorResult<ProjectWorkflowOutcome> {
    let project_root = request.path.clone();
    az_project_scaffold::new::execute_with_options(
        request.name.clone(),
        Some(project_root.clone()),
        az_project_scaffold::new::ProjectCreateOptions {
            lore_url: request.lore_url.clone(),
            enabled_engine_gems: request.enabled_gems.clone(),
            topology: request.topology,
        },
    )?;

    info!(
        project = %request.name,
        root = %project_root.display(),
        "created project through shared scaffold workflow"
    );
    Ok(ProjectWorkflowOutcome {
        operation: ui::Operation::CreateProject,
        next_steps: project_workflow_next_steps(&project_root),
        project_root,
        message: format!("Created project `{}`", request.name),
    })
}

/// A request to scaffold a new project gem, resolved from a [`CreateGem`]
/// action plus the attached project.
///
/// [`CreateGem`]: az_editor_ui::actions::CreateGem
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectWorkflowCreateGemRequest {
    pub project_root: PathBuf,
    pub name: String,
    pub id: Option<String>,
    pub capabilities: Vec<String>,
    pub register: bool,
}

/// Scaffold a new project gem through the shared `az_project_scaffold::gem`
/// workflow.
///
/// This is the same entry point `azoth gem new` uses, including the ADR 0026
/// capability templates. Runs in-process from the editor, mirroring
/// [`create_project`]/[`initialize_project`].
#[instrument(skip_all, fields(gem = %request.name, root = %request.project_root.display()))]
/// Scaffold a new gem inside an attached project.
///
/// # Errors
///
/// Returns [`EditorError::ProjectWorkflow`] if
/// [`az_project_scaffold::gem::new_gem`] fails — the gem name or id is already
/// taken, `gems/<name>` cannot be written, or registering the gem in the
/// project manifest fails.
pub fn create_gem(request: &ProjectWorkflowCreateGemRequest) -> EditorResult<String> {
    az_project_scaffold::gem::new_gem(
        Some(request.project_root.clone()),
        request.name.clone(),
        // Default gem path (`gems/<name>`) and package name (`<name>`) — the
        // editor dialog does not expose these overrides.
        None,
        request.id.clone(),
        None,
        request.register,
        &request.capabilities,
    )?;

    info!(
        gem = %request.name,
        root = %request.project_root.display(),
        register = request.register,
        capabilities = ?request.capabilities,
        "created gem through shared scaffold workflow"
    );
    Ok(format!("Created gem `{}`", request.name))
}

/// Initialize the Azoth project workflow in an existing directory.
///
/// # Errors
///
/// Returns [`EditorError::ProjectWorkflow`] if
/// [`az_project_scaffold::init::execute`] fails — the directory already holds
/// a project manifest, it is not writable, or the Lore repository URL is
/// rejected.
pub fn initialize_project(
    request: &ProjectWorkflowInitRequest,
) -> EditorResult<ProjectWorkflowOutcome> {
    let project_root = request.path.clone();
    az_project_scaffold::init::execute(
        Some(project_root.clone()),
        request.name.clone(),
        request.lore_url.clone(),
    )?;

    info!(
        root = %project_root.display(),
        name = ?request.name,
        "initialized project through shared scaffold workflow"
    );
    Ok(ProjectWorkflowOutcome {
        operation: ui::Operation::InitializeProject,
        next_steps: project_workflow_next_steps(&project_root),
        project_root,
        message: "Initialized Azoth project workflow".to_string(),
    })
}

fn create_request_from_action(
    action: &az_editor_ui::actions::CreateProject,
) -> EditorResult<ProjectWorkflowCreateRequest> {
    let name = required_text("project name", &action.name)?;
    let path = optional_text(&action.path).map_or_else(|| PathBuf::from(&name), PathBuf::from);
    Ok(ProjectWorkflowCreateRequest {
        name,
        path,
        lore_url: action.lore_url.as_deref().and_then(optional_text),
        topology: parse_project_topology(&action.topology)?,
        enabled_gems: normalized_enabled_gems(&action.enabled_gems),
    })
}

fn create_gem_request_from_action(
    action: &az_editor_ui::actions::CreateGem,
    cx: &App,
) -> EditorResult<ProjectWorkflowCreateGemRequest> {
    let project_root = cx
        .try_global::<EditorAttachSession>()
        .map(|session| session.project_root.clone())
        .ok_or_else(|| {
            EditorError::InvalidArgument(
                "no attached project; open a project before creating a gem".to_string(),
            )
        })?;
    let name = required_text("gem name", &action.name)?;
    let id = action.id.as_deref().and_then(optional_text);
    let capabilities = action
        .capability
        .as_deref()
        .and_then(optional_text)
        .into_iter()
        .collect();
    Ok(ProjectWorkflowCreateGemRequest {
        project_root,
        name,
        id,
        capabilities,
        register: action.register,
    })
}

fn publish_gem_creation_success(cx: &mut App, message: String) {
    info!(%message, "gem workflow create succeeded");
    publish_console_log(cx, LogLevel::Info, "gem-workflow", message.clone());
    // A newly scaffolded gem is registered in the project manifest and Cargo
    // wiring, but only loads after a rebuild — surface the same staged-change
    // banner the Gems panel already uses for enable/disable edits.
    cx.set_global(az_editor_ui::EditorGemRebuildState {
        rebuild_pending: true,
    });
    cx.set_global(az_editor_ui::EditorGemCreationStatus::success(message));
    cx.refresh_windows();
}

fn publish_gem_creation_error(cx: &mut App, err: &EditorError) {
    let message = err.to_string();
    error!(error = %err, "gem workflow create failed");
    publish_console_log(cx, LogLevel::Error, "gem-workflow", message.clone());
    cx.set_global(az_editor_ui::EditorGemCreationStatus::error(message));
    cx.refresh_windows();
}

fn parse_project_topology(value: &str) -> EditorResult<az_project_scaffold::ProjectTopologyKind> {
    match value.trim() {
        "single-player" => Ok(az_project_scaffold::ProjectTopologyKind::SinglePlayer),
        "multiplayer-client-server" => {
            Ok(az_project_scaffold::ProjectTopologyKind::MultiplayerClientServer)
        }
        "multiplayer-peer-to-peer" => {
            Ok(az_project_scaffold::ProjectTopologyKind::MultiplayerPeerToPeer)
        }
        value => Err(EditorError::InvalidArgument(format!(
            "project topology `{value}` is invalid; expected single-player, multiplayer-client-server, or multiplayer-peer-to-peer"
        ))),
    }
}

fn normalized_enabled_gems(gems: &[String]) -> Vec<String> {
    gems.iter()
        .filter_map(|gem| optional_text(gem))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn init_request_from_action(
    action: &az_editor_ui::actions::InitializeProjectWorkflow,
) -> EditorResult<ProjectWorkflowInitRequest> {
    Ok(ProjectWorkflowInitRequest {
        path: PathBuf::from(required_text("project path", &action.path)?),
        name: action.name.as_deref().and_then(optional_text),
        lore_url: action.lore_url.as_deref().and_then(optional_text),
    })
}

fn ensure_session_request_from_action(
    action: &az_editor_ui::actions::EnsureProjectWorkflowSession,
) -> EditorResult<ProjectWorkflowSessionRequest> {
    Ok(ProjectWorkflowSessionRequest {
        project_root: PathBuf::from(required_text("project root", &action.project_root)?),
        session_name: required_text("session name", &action.session_name)?,
    })
}

fn prepare_services_request_from_action(
    action: &az_editor_ui::actions::PrepareProjectWorkflowSessionServices,
) -> EditorResult<ProjectWorkflowServicePrepareRequest> {
    Ok(ProjectWorkflowServicePrepareRequest {
        project_root: PathBuf::from(required_text("project root", &action.project_root)?),
        session_slug: required_text("session slug", &action.session_slug)?,
    })
}

fn open_editor_session_request_from_action(
    action: &az_editor_ui::actions::OpenProjectWorkflowEditorSession,
) -> EditorResult<ProjectWorkflowEditorSessionOpenRequest> {
    Ok(ProjectWorkflowEditorSessionOpenRequest {
        project_root: PathBuf::from(required_text("project root", &action.project_root)?),
        session_name: required_text("session name", &action.session_name)?,
    })
}

/// Validate that `project_root` is a real Azoth project by loading and
/// validating its `azoth.toml` manifest. Reuses the shared project manifest
/// loader/validator; any read/parse/validation failure becomes an editor error.
fn validate_project_root_for_open(
    project_root: &std::path::Path,
) -> EditorResult<az_project_scaffold::ProjectSummary> {
    az_project_scaffold::load_project_summary(project_root).map_err(|error| {
        EditorError::InvalidArgument(format!(
            "`{}` is not a valid Azoth project: {error}",
            project_root.display()
        ))
    })
}

/// Record a validated project in the recent-opened store immediately, before
/// the build/attach completes.
fn record_recent_project_from_manifest(
    project_root: &std::path::Path,
    manifest: &az_project_scaffold::ProjectSummary,
) {
    crate::recent_projects::record_recent_project(crate::recent_projects::RecentProjectEntry {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        path: project_root.to_string_lossy().into_owned(),
        engine_version: manifest.engine_version.clone(),
        last_opened_unix_ms: 0,
    });
}

/// Register the project with azd and ensure the named session exists.
///
/// # Errors
///
/// Returns any error [`ensure_daemon_endpoint_for_project`] returns if azd
/// cannot be located or started for `project_root`, [`EditorError::Io`] if the
/// current-thread runtime cannot be built, or [`EditorError::RpcTransport`] if
/// connecting to azd, registering the project root, or ensuring the session
/// fails.
pub fn ensure_project_workflow_session(
    request: ProjectWorkflowSessionRequest,
) -> EditorResult<ProjectWorkflowSessionOutcome> {
    let project_root = request.project_root.clone();
    let daemon_endpoint = ensure_daemon_endpoint_for_project(&project_root)?;

    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();
    local.block_on(&runtime, async move {
        let daemon = AzDaemonClient::connect(&daemon_endpoint).await?;
        let project = daemon.register_project_root(&project_root).await?;
        let session = daemon
            .ensure_project_session(&project.project_id, &request.session_name)
            .await?;
        let manifest = session.manifest;
        info!(
            project_id = %project.project_id,
            session = %manifest.slug,
            workspace = %manifest.workspace_root,
            created = session.created,
            "ensured project workflow session through azd"
        );
        Ok(ProjectWorkflowSessionOutcome {
            project_root,
            project_id: project.project_id,
            session_slug: manifest.slug,
            workspace_root: manifest.workspace_root,
            created: session.created,
        })
    })
}

/// Start the session's project services through azd and report their
/// endpoints.
///
/// # Errors
///
/// Returns any error [`ensure_daemon_endpoint_for_project`] returns if azd
/// cannot be located or started for the project, [`EditorError::Io`] if the
/// current-thread runtime cannot be built, or [`EditorError::RpcTransport`] if
/// connecting to azd, registering the project root, or preparing the session
/// services fails.
pub fn prepare_project_workflow_session_services(
    request: ProjectWorkflowServicePrepareRequest,
) -> EditorResult<ProjectWorkflowServicePrepareOutcome> {
    let project_root = request.project_root.clone();
    let daemon_endpoint = ensure_daemon_endpoint_for_project(&project_root)?;

    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();
    local.block_on(&runtime, async move {
        let daemon = AzDaemonClient::connect(&daemon_endpoint).await?;
        let project = daemon.register_project_root(&project_root).await?;
        let services = daemon
            .prepare_project_session_services(
                &project.project_id,
                &request.session_slug,
                default_project_service_endpoint_kind(),
                false,
            )
            .await?;
        info!(
            project_id = %project.project_id,
            session = %services.manifest.slug,
            services = ?services.service_names,
            built = services.built,
            "prepared project workflow session services through azd"
        );
        Ok(ProjectWorkflowServicePrepareOutcome {
            project_root,
            project_id: project.project_id,
            session_slug: services.manifest.slug,
            service_names: services.service_names,
            built: services.built,
        })
    })
}

/// Bootstrap azd for the project, then ensure, start and attach its editor
/// session.
///
/// # Errors
///
/// Returns any error [`ensure_daemon_endpoint_for_project`] returns if azd
/// cannot be located or started, followed by any error
/// [`open_project_workflow_editor_session_with_daemon_endpoint`] returns.
pub fn open_project_workflow_editor_session(
    request: ProjectWorkflowEditorSessionOpenRequest,
) -> EditorResult<ProjectWorkflowEditorSessionOpenOutcome> {
    let started = Instant::now();
    let bootstrap_started = Instant::now();
    let daemon_endpoint = ensure_daemon_endpoint_for_project(&request.project_root)?;
    let bootstrap_ms = bootstrap_started.elapsed().as_millis();
    let result =
        open_project_workflow_editor_session_with_daemon_endpoint(request, daemon_endpoint, None);
    info!(
        total_ms = started.elapsed().as_millis(),
        bootstrap_ms, "editor project open including daemon bootstrap completed"
    );
    result
}

/// Open with editor-visible progress milestones. Used by the foreground task to
/// keep the launcher/status line on the actual service-start and attach state.
fn open_project_workflow_editor_session_with_progress(
    request: ProjectWorkflowEditorSessionOpenRequest,
    progress_tx: &UnboundedSender<OpenProgressUpdate>,
) -> EditorResult<ProjectWorkflowEditorSessionOpenOutcome> {
    let daemon_endpoint = ensure_daemon_endpoint_for_project(&request.project_root)?;
    open_project_workflow_editor_session_with_daemon_endpoint(
        request,
        daemon_endpoint,
        Some(progress_tx),
    )
}

#[instrument(
    skip_all,
    fields(
        project_root = %request.project_root.display(),
        session = %request.session_name,
        endpoint_kind = ?daemon_endpoint.kind
    )
)]
/// Ensure the session and its project services through an already-resolved azd
/// endpoint, then attach the editor to it.
///
/// # Errors
///
/// Returns [`EditorError::Io`] if the current-thread runtime cannot be built,
/// [`EditorError::RpcTransport`] if connecting to azd, registering the project
/// root, ensuring the session, or starting its project services fails,
/// [`EditorError::ServiceDiscovery`] if the started services do not publish
/// the endpoint the editor attaches to, and any error
/// [`crate::attach_to_session_via_daemon`] returns while attaching.
pub fn open_project_workflow_editor_session_with_daemon_endpoint(
    request: ProjectWorkflowEditorSessionOpenRequest,
    daemon_endpoint: Endpoint,
    // When present, the editor publishes authoritative open milestones into
    // this channel. The service-start RPC result, not a reverse progress
    // callback, is the attach gate.
    progress_tx: Option<&UnboundedSender<OpenProgressUpdate>>,
) -> EditorResult<ProjectWorkflowEditorSessionOpenOutcome> {
    let open_started = Instant::now();
    let project_root = request.project_root.clone();
    let runtime = Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()?;
    let local = LocalSet::new();
    let service_ensure_started = Instant::now();
    let service_outcome = local.block_on(&runtime, {
        let project_root = project_root.clone();
        let session_name = request.session_name;
        let daemon_endpoint = daemon_endpoint.clone();
        let progress_tx = progress_tx.cloned();
        async move {
            let daemon = AzDaemonClient::connect(&daemon_endpoint).await?;
            let project = daemon.register_project_root(&project_root).await?;
            let required_services = editor_required_session_service_names();
            if let Some(tx) = progress_tx.as_ref() {
                let _ = tx.send(OpenProgressUpdate {
                    seq: 1,
                    phase: ui::OpenPhase::StartServices,
                    done_bp: 0,
                    phase_done: 0,
                    phase_total: Some(required_services.len() as u64),
                    message: "starting services".to_string(),
                });
            }
            info!(
                project_id = %project.project_id,
                session = %session_name,
                required_services = ?required_services,
                "ensuring project workflow services through azd"
            );
            let services = daemon
                .ensure_project_session_services_with_progress(
                    &project.project_id,
                    &session_name,
                    default_project_service_endpoint_kind(),
                    false,
                    required_services,
                    EDITOR_SESSION_SERVICE_START_TIMEOUT_MS,
                    &daemon_endpoint,
                    project_open_progress::NoopProjectOpenProgressSink.into_client(),
                )
                .await?;
            info!(
                project_id = %project.project_id,
                session = %services.manifest.slug,
                running_services = ?services.running_service_names,
                "ensured project workflow services through azd"
            );
            Ok::<_, EditorError>((project, services))
        }
    })?;
    let service_ensure_ms = service_ensure_started.elapsed().as_millis();
    let (project, services) = service_outcome;

    publish_open_services_started(progress_tx, services.running_service_names.len());
    let attach_started = Instant::now();
    let attach_session = attach_to_session_via_daemon(
        &project_root,
        Some(&services.manifest.slug),
        daemon_endpoint,
    )?;
    let attach_ms = attach_started.elapsed().as_millis();
    publish_open_attach_completed(progress_tx);
    log_open_project_timing(
        &project.project_id,
        &services.manifest.slug,
        open_started.elapsed().as_millis(),
        service_ensure_ms,
        attach_ms,
    );

    Ok(ProjectWorkflowEditorSessionOpenOutcome {
        project_root,
        project_id: project.project_id,
        session_slug: services.manifest.slug,
        running_service_names: services.running_service_names,
        attach_session,
    })
}

/// The daemon RPC result is the source of truth that required services are
/// running. Publishing that completion locally guards against stale or delayed
/// progress callbacks leaving the UI on an old 0/N service state.
fn publish_open_services_started(
    progress_tx: Option<&UnboundedSender<OpenProgressUpdate>>,
    running_services: usize,
) {
    let Some(tx) = progress_tx else {
        return;
    };
    let running_count = running_services as u64;
    let total_count = (editor_required_session_service_names().len() as u64).max(running_count);
    let _ = tx.send(OpenProgressUpdate {
        seq: u64::MAX - 3,
        phase: ui::OpenPhase::StartServices,
        done_bp: project_open_progress::START_SERVICES_DONE_BP,
        phase_done: running_count,
        phase_total: Some(total_count),
        message: format!("{running_count} services running"),
    });
    let _ = tx.send(OpenProgressUpdate {
        seq: u64::MAX - 2,
        phase: ui::OpenPhase::Attach,
        done_bp: project_open_progress::START_SERVICES_DONE_BP,
        phase_done: 0,
        phase_total: Some(1),
        message: "attaching to session".to_string(),
    });
}

/// Publish the terminal attach and catalog-load milestones once the editor is
/// attached to the session.
fn publish_open_attach_completed(progress_tx: Option<&UnboundedSender<OpenProgressUpdate>>) {
    let Some(tx) = progress_tx else {
        return;
    };
    let _ = tx.send(OpenProgressUpdate::editor_phase(
        u64::MAX - 1,
        ui::OpenPhase::Attach,
        project_open_progress::ATTACH_DONE_BP,
        "session attached",
    ));
    let _ = tx.send(OpenProgressUpdate::editor_phase(
        u64::MAX,
        ui::OpenPhase::LoadCatalogs,
        project_open_progress::LOAD_CATALOGS_DONE_BP,
        "catalogs loaded",
    ));
}

/// Emit the one timing table summarizing an editor project open.
fn log_open_project_timing(
    project_id: &str,
    session_slug: &str,
    total_ms: u128,
    service_ensure_ms: u128,
    attach_ms: u128,
) {
    info!(
        project_id = %project_id,
        session = %session_slug,
        total_ms,
        service_ensure_ms,
        attach_ms,
        timing_table = %format!(
            "stage                 ms\nservice ensure     {service_ensure_ms:>6}\nattach+verify      {attach_ms:>6}\ntotal              {total_ms:>6}"
        ),
        "editor project open timing summary"
    );
}

/// Publishes all verified session globals and the single controller aggregate.
/// This is the only attached-session installer used by startup, project open,
/// and recovery reattach paths.
pub(crate) fn install_verified_attached_session(
    cx: &mut App,
    session: EditorAttachSession,
) -> EditorResult<()> {
    // The globals installer takes ownership -- the session becomes a global --
    // so the controller plan gets its own copy to hand to each installer.
    let controller_session = session.clone();
    install_project_workflow_attached_session_globals(cx, session);
    install_attached_controllers(cx, &controller_session).map_err(|error| {
        EditorError::ServiceDiscovery(format!(
            "invalid attached editor controller install plan: {error}"
        ))
    })?;
    if cx.try_global::<OpenAssetProcessorAfterAttach>().is_some() {
        cx.remove_global::<OpenAssetProcessorAfterAttach>();
        if let Err(error) = open_or_focus_asset_processor_window(cx) {
            error!(%error, "failed to open pending asset processor window after project attach");
        }
    }
    Ok(())
}

fn install_project_workflow_attached_session_globals(cx: &mut App, session: EditorAttachSession) {
    info!(
        project_id = %session.project_id,
        session_slug = %session.session_slug,
        project_root = %session.project_root.display(),
        workspace_root = %session.workspace.workspace_root,
        "project workflow attached session globals installed"
    );

    crate::ui_state_persistence::install_for_attached_project(&session.project_root, cx);
    crate::workspace::dock::restore_cached_asset_browser_from_project_state(cx);

    cx.set_global(EditorTypeRegistry::new(session.type_registry.clone()));
    cx.set_global(EditorAddableAuthoredComponents::new(
        addable_reflected_component_data(&session.type_registry),
    ));
    cx.set_global(EditorGameDataCatalog::new(session.gamedata_catalog.clone()));
    cx.set_global(EditorCreatableAuthoredSchemas::new(Vec::new()));
    cx.set_global(gem_selection_from_project_inventory(
        &session.project_inventory,
    ));
    cx.set_global(session);
}

fn publish_project_workflow_outcome(cx: &mut App, outcome: ProjectWorkflowOutcome) {
    let root = path_label(&outcome.project_root);
    for step in &outcome.next_steps {
        publish_console_log(
            cx,
            LogLevel::Info,
            "project-workflow",
            format!("{}: {}", step.label, step.command),
        );
    }
    publish_console_log(
        cx,
        LogLevel::Info,
        "project-workflow",
        outcome.message.clone(),
    );
    publish_project_workflow_status(
        cx,
        ui::Status::succeeded(outcome.operation, root, outcome.message, outcome.next_steps),
    );
}

fn publish_project_workflow_error(
    cx: &mut App,
    operation: ui::Operation,
    project_root: impl Into<String>,
    err: &EditorError,
) {
    let project_root = project_root.into();
    let message = err.to_string();
    let next_steps = project_workflow_error_next_steps(&message, &project_root);
    error!(error = %err, operation = ?operation, "project workflow action failed");
    publish_console_log(cx, LogLevel::Error, "project-workflow", message.clone());
    publish_project_workflow_status(
        cx,
        ui::Status::failed_with_next_steps(operation, project_root, message, next_steps),
    );
}

fn publish_console_log(
    cx: &mut App,
    level: LogLevel,
    source: &'static str,
    message: impl Into<String>,
) {
    let message = message.into();
    cx.default_global::<ConsoleState>()
        .log_from_source(level, source, message);
}

fn publish_project_workflow_status(cx: &mut App, status: ui::Status) {
    info!(
        mode = ?status.mode,
        phase = ?status.phase,
        operation = ?status.operation,
        project_root = ?status.project_root,
        attached_session = status.attached_session.is_some(),
        "project workflow status published"
    );
    cx.set_global(status);
    cx.refresh_windows();
}

fn project_workflow_operation_running(cx: &App, operation: ui::Operation) -> bool {
    cx.try_global::<ui::Status>().is_some_and(|status| {
        status.phase == ui::Phase::Running && status.operation == Some(operation)
    })
}

fn current_project_workflow_steps_after_session_bootstrap(cx: &App) -> Vec<ui::NextStep> {
    cx.try_global::<ui::Status>()
        .map(|status| {
            status
                .next_steps
                .iter()
                .filter(|step| step.kind != ui::NextStepKind::CreateMainSession)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn current_project_workflow_steps_after_service_preparation(cx: &App) -> Vec<ui::NextStep> {
    cx.try_global::<ui::Status>()
        .map(|status| {
            status
                .next_steps
                .iter()
                .filter(|step| step.kind != ui::NextStepKind::OpenEditorSession)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

const fn default_project_service_endpoint_kind() -> EndpointKind {
    if cfg!(windows) {
        EndpointKind::WindowsNamedPipe
    } else {
        EndpointKind::UnixDomainSocket
    }
}

fn required_text(label: &'static str, value: &str) -> EditorResult<String> {
    optional_text(value).ok_or_else(|| {
        EditorError::InvalidArgument(format!("project workflow requires non-empty {label}"))
    })
}

fn optional_text(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn path_label(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

fn project_workflow_next_steps(project_root: &std::path::Path) -> Vec<ui::NextStep> {
    let source_control_state =
        az_project_scaffold::new::project_workflow_source_control_state(project_root)
            .unwrap_or_else(
                |_| az_project_scaffold::new::ProjectWorkflowSourceControlState {
                    has_lore_repository: project_root.join(".lore").is_dir(),
                    has_committed_revision: false,
                    has_local_changes: false,
                },
            );
    project_workflow_next_steps_for_source_control_state(source_control_state, project_root)
}

fn project_workflow_next_steps_for_source_control_state(
    source_control_state: az_project_scaffold::new::ProjectWorkflowSourceControlState,
    project_root: &std::path::Path,
) -> Vec<ui::NextStep> {
    az_project_scaffold::new::project_workflow_next_step_plan(
        source_control_state,
        az_project_scaffold::new::INITIAL_PROJECT_COMMIT_MESSAGE,
        Some(project_root),
    )
    .steps
    .into_iter()
    .map(|step| {
        ui::NextStep::new(
            project_workflow_next_step_kind_to_ui(step.kind),
            step.label,
            step.command,
        )
    })
    .collect()
}

fn project_workflow_error_next_steps(message: &str, project_root: &str) -> Vec<ui::NextStep> {
    if !message.contains("project session base ref `HEAD` is not a valid commit") {
        return Vec::new();
    }
    let project_root = project_root.trim();
    if project_root.is_empty() {
        return Vec::new();
    }
    project_workflow_next_steps(std::path::Path::new(project_root))
}

const fn project_workflow_next_step_kind_to_ui(
    kind: az_project_scaffold::new::ProjectWorkflowNextStepKind,
) -> ui::NextStepKind {
    match kind {
        az_project_scaffold::new::ProjectWorkflowNextStepKind::CreateLoreRepository => {
            ui::NextStepKind::CreateLoreRepository
        }
        az_project_scaffold::new::ProjectWorkflowNextStepKind::CommitProjectWorkflow => {
            ui::NextStepKind::CommitProjectWorkflow
        }
        az_project_scaffold::new::ProjectWorkflowNextStepKind::CreateMainSession => {
            ui::NextStepKind::CreateMainSession
        }
        az_project_scaffold::new::ProjectWorkflowNextStepKind::OpenEditorSession => {
            ui::NextStepKind::OpenEditorSession
        }
        az_project_scaffold::new::ProjectWorkflowNextStepKind::InspectSessionServices => {
            ui::NextStepKind::InspectSessionServices
        }
        az_project_scaffold::new::ProjectWorkflowNextStepKind::RunProject => {
            ui::NextStepKind::RunProject
        }
    }
}

impl ProjectWorkflowCreateRequest {
    fn project_root_label(&self) -> String {
        path_label(&self.path)
    }
}

impl ProjectWorkflowInitRequest {
    fn project_root_label(&self) -> String {
        path_label(&self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_defaults_path_to_project_name() {
        let request = create_request_from_action(&az_editor_ui::actions::CreateProject {
            name: "sample-game".to_string(),
            path: String::new(),
            lore_url: Some(" lore://127.0.0.1:41337 ".to_string()),
            topology: "multiplayer-peer-to-peer".to_string(),
            enabled_gems: vec![
                " azoth.gamedata ".to_string(),
                String::new(),
                "azoth.audio-system".to_string(),
                "azoth.gamedata".to_string(),
            ],
        })
        .unwrap();

        assert_eq!(request.name, "sample-game");
        assert_eq!(request.path, PathBuf::from("sample-game"));
        assert_eq!(request.lore_url.as_deref(), Some("lore://127.0.0.1:41337"));
        assert_eq!(
            request.topology,
            az_project::ProjectTopologyKind::MultiplayerPeerToPeer
        );
        assert_eq!(
            request.enabled_gems,
            vec![
                "azoth.audio-system".to_string(),
                "azoth.gamedata".to_string()
            ]
        );
    }

    #[test]
    fn init_request_trims_optional_name_and_lore_url() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        let request = init_request_from_action(&az_editor_ui::actions::InitializeProjectWorkflow {
            path: format!(" {} ", project_root.display()),
            name: Some(" Sample Game ".to_string()),
            lore_url: Some(" lore://127.0.0.1:41337 ".to_string()),
        })
        .unwrap();

        assert_eq!(request.path, project_root);
        assert_eq!(request.name.as_deref(), Some("Sample Game"));
        assert_eq!(request.lore_url.as_deref(), Some("lore://127.0.0.1:41337"));
    }

    #[test]
    fn ensure_session_request_trims_project_root_and_session() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        let request = ensure_session_request_from_action(
            &az_editor_ui::actions::EnsureProjectWorkflowSession {
                project_root: format!(" {} ", project_root.display()),
                session_name: " main ".to_string(),
            },
        )
        .unwrap();

        assert_eq!(request.project_root, project_root);
        assert_eq!(request.session_name, "main");
    }

    #[test]
    fn ensure_session_request_rejects_empty_project_root() {
        let error = ensure_session_request_from_action(
            &az_editor_ui::actions::EnsureProjectWorkflowSession {
                project_root: String::new(),
                session_name: "main".to_string(),
            },
        )
        .unwrap_err();

        assert!(error.to_string().contains("project root"));
    }

    #[test]
    fn prepare_services_request_trims_project_root_and_session_slug() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        let request = prepare_services_request_from_action(
            &az_editor_ui::actions::PrepareProjectWorkflowSessionServices {
                project_root: format!(" {} ", project_root.display()),
                session_slug: " main ".to_string(),
            },
        )
        .unwrap();

        assert_eq!(request.project_root, project_root);
        assert_eq!(request.session_slug, "main");
    }

    #[test]
    fn editor_open_requires_only_editor_session_services() {
        assert_eq!(
            editor_required_session_service_names(),
            vec![
                PROJECT_HOST_SERVICE_NAME.to_string(),
                ASSET_PROCESSOR_SERVICE_NAME.to_string(),
                ASSET_WORKER_SERVICE_NAME.to_string(),
            ]
        );
    }

    #[test]
    fn create_request_rejects_empty_project_name() {
        let error = create_request_from_action(&az_editor_ui::actions::CreateProject {
            name: String::new(),
            path: String::new(),
            lore_url: None,
            topology: "single-player".to_string(),
            enabled_gems: Vec::new(),
        })
        .unwrap_err();

        assert!(error.to_string().contains("project name"));
    }

    #[test]
    fn creates_project_through_shared_scaffold() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");

        let outcome = create_project(&ProjectWorkflowCreateRequest {
            name: "sample-game".to_string(),
            path: project_root.clone(),
            lore_url: None,
            topology: az_project::ProjectTopologyKind::SinglePlayer,
            enabled_gems: vec![
                "azoth.gamedata".to_string(),
                "azoth.audio-system".to_string(),
            ],
        })
        .unwrap();

        assert_eq!(outcome.operation, ui::Operation::CreateProject);
        assert_eq!(outcome.project_root, project_root);
        assert!(outcome.message.contains("sample-game"));
        assert_eq!(
            outcome
                .next_steps
                .iter()
                .map(|step| (step.kind, step.label.as_str()))
                .collect::<Vec<_>>(),
            vec![
                (
                    ui::NextStepKind::CreateLoreRepository,
                    "Create Lore repository",
                ),
                (
                    ui::NextStepKind::CommitProjectWorkflow,
                    "Commit project workflow",
                ),
                (ui::NextStepKind::CreateMainSession, "Create main session",),
                (ui::NextStepKind::OpenEditorSession, "Open editor session",),
                (
                    ui::NextStepKind::InspectSessionServices,
                    "Inspect session services",
                ),
                (ui::NextStepKind::RunProject, "Run project"),
            ]
        );
        assert!(
            outcome
                .next_steps
                .iter()
                .any(|step| step.command.contains(&format!(
                    "azoth editor --session main --project {}",
                    project_root.display()
                )))
        );
        assert!(outcome.project_root.join("azoth.toml").exists());
        assert!(outcome.project_root.join("Cargo.toml").exists());
        assert!(
            outcome
                .project_root
                .join("gems/sample-game/runtime/Cargo.toml")
                .exists()
        );

        let manifest = az_project::load_project_manifest(&outcome.project_root).unwrap();
        // The scaffold records the requested engine gems (path-less references)
        // alongside the topology's path-backed primary project gem (ADR-0025).
        let enabled_engine_gems = manifest
            .gems
            .iter()
            .filter(|gem| gem.path.is_none())
            .map(|gem| (gem.id.as_str(), gem.enabled))
            .collect::<Vec<_>>();
        assert_eq!(
            enabled_engine_gems,
            vec![("azoth.audio-system", true), ("azoth.gamedata", true)]
        );
        assert!(
            manifest
                .gems
                .iter()
                .any(|gem| gem.path.is_some() && gem.enabled),
            "scaffold must record the topology's primary project gem"
        );
    }

    #[test]
    fn creates_and_registers_capability_gem_through_shared_scaffold() {
        let temp = tempfile::tempdir().unwrap();
        let project_root = temp.path().join("sample-game");
        create_project(&ProjectWorkflowCreateRequest {
            name: "sample-game".to_string(),
            path: project_root.clone(),
            lore_url: None,
            topology: az_project::ProjectTopologyKind::SinglePlayer,
            enabled_gems: Vec::new(),
        })
        .unwrap();

        // A blank id + a session-authority capability, register on by default.
        let message = create_gem(&ProjectWorkflowCreateGemRequest {
            project_root: project_root.clone(),
            name: "combat-system".to_string(),
            id: None,
            capabilities: vec!["session-authority".to_string()],
            register: true,
        })
        .unwrap();

        assert!(message.contains("combat-system"));
        let gem_root = project_root.join("gems/combat-system");
        assert!(gem_root.join("gem.toml").exists());
        assert!(gem_root.join("src/lib.rs").exists());

        // The id is derived from the name and the gem is registered + enabled.
        let manifest = az_project::load_project_manifest(&project_root).unwrap();
        let gem = manifest
            .gems
            .iter()
            .find(|gem| gem.id == "local.combat_system")
            .expect("new gem registered in project manifest");
        assert!(gem.enabled);
        assert!(gem.path.is_some());
    }

    #[test]
    fn open_project_action_uses_background_workflow_task() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("project_workflow.rs"),
        )
        .expect("read project_workflow.rs");
        let action_start = source
            .find("az_editor_ui::actions::OpenProjectWorkflowEditorSession")
            .expect("find open project action handler");
        let action_end = source[action_start..]
            .find("\n        },\n    );")
            .map(|offset| action_start + offset)
            .expect("find open project action handler end");
        let action_source = &source[action_start..action_end];
        let open_helper_start = source
            .find("fn open_project_workflow_editor_session_from_action(")
            .expect("find open project action helper");
        let open_helper_end = source[open_helper_start..]
            .find("\nfn spawn_open_project_workflow_editor_session(")
            .map(|offset| open_helper_start + offset)
            .expect("find open project action helper end");
        let open_helper_source = &source[open_helper_start..open_helper_end];
        let helper_start = source
            .find("fn spawn_open_project_workflow_editor_session(")
            .expect("find open project background helper");
        let helper_end = source[helper_start..]
            .find("\npub fn create_project(")
            .map(|offset| helper_start + offset)
            .expect("find helper end");
        let helper_source = &source[helper_start..helper_end];

        assert!(sym(
            action_source,
            "open_project_workflow_editor_session_from_action"
        ));
        assert!(sym(
            open_helper_source,
            "spawn_open_project_workflow_editor_session"
        ));
        assert!(!sym(
            action_source,
            "open_project_workflow_editor_session(request)"
        ));
        assert!(sym(helper_source, "cx.background_executor().clone()"));
        assert!(sym(helper_source, "background.spawn(async move"));
        assert!(sym(
            helper_source,
            "open_project_workflow_editor_session_with_daemon_endpoint"
        ));
        assert!(sym(
            helper_source,
            "open_project_workflow_editor_session_with_progress"
        ));
        assert!(sym(helper_source, "tokio::select!"));
        assert!(sym(helper_source, "cx.update(move |cx|"));
    }

    #[test]
    fn editor_open_attach_is_gated_by_service_start_result_not_progress_callback() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("project_workflow.rs"),
        )
        .expect("read project_workflow.rs");
        let open_start = source
            .find("pub fn open_project_workflow_editor_session_with_daemon_endpoint(")
            .expect("find open project daemon helper");
        let open_end = source[open_start..]
            .find("\n/// Publishes all verified session globals")
            .map(|offset| open_start + offset)
            .expect("find open project daemon helper end");
        let open_source = &source[open_start..open_end];

        assert!(sym(
            open_source,
            ".ensure_project_session_services_with_progress("
        ));
        assert!(sym(
            open_source,
            "NoopProjectOpenProgressSink.into_client()"
        ));
        // (A third pin here used to assert a *comment* mentioning the
        // "service-start RPC result"; symbol checks ignore comments, and the
        // structural pins above already enforce the progress-sink contract.)
    }

    /// True when `snippet` appears in `source` as symbols, ignoring
    /// formatting and comments (ticket 012).
    fn sym(source: &str, snippet: &str) -> bool {
        az_architecture_guard::symbols_contain(source, snippet)
    }

    /// Symbol-skeleton position of `snippet` in `source`. Ordering guards
    /// compare these instead of raw byte offsets so reformatting a pinned
    /// region cannot silently move a pin (ticket 012).
    fn sym_pos(source: &str, snippet: &str) -> Option<usize> {
        az_architecture_guard::symbol_skeleton(source)
            .find(&az_architecture_guard::symbol_skeleton(snippet))
    }

    /// Isolate the background-task body of
    /// `spawn_open_project_workflow_editor_session_with_daemon_endpoint`: the
    /// single place every open-workflow outcome (success, service-start
    /// failure, attach failure, or a raw daemon RPC failure) is observed and
    /// turned into a connection-state transition.
    fn open_background_task_source() -> String {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("project_workflow.rs"),
        )
        .expect("read project_workflow.rs");
        let start = source
            .find("pub(crate) fn spawn_open_project_workflow_editor_session_with_daemon_endpoint(")
            .expect("find open project background task fn");
        let end = source[start..]
            .find("\nfn publish_open_project_progress_update(")
            .map(|offset| start + offset)
            .expect("find open project background task fn end");
        source[start..end].to_string()
    }

    #[test]
    fn open_action_sets_connecting_state_before_spawning_background_task() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("project_workflow.rs"),
        )
        .expect("read project_workflow.rs");
        let fn_start = source
            .find("pub(crate) fn open_project_workflow_editor_session_from_action(")
            .expect("find open project action handler");
        let fn_end = source[fn_start..]
            .find("\nfn spawn_open_project_workflow_editor_session(")
            .map(|offset| fn_start + offset)
            .expect("find open project action handler end");
        let fn_source = &source[fn_start..fn_end];

        let connecting_pos = sym_pos(
            fn_source,
            "cx.set_global(EditorProjectConnectionState::connecting());",
        )
        .expect("open action must set EditorProjectConnectionState::connecting()");
        let spawn_pos = sym_pos(
            fn_source,
            "spawn_open_project_workflow_editor_session(cx, request, project_root, remaining_steps);",
        )
        .expect("open action must spawn the background open task");
        assert!(
            connecting_pos < spawn_pos,
            "connection state must be set to Connecting before the background open task is \
             spawned, so gated panels never render live/stale data during an in-flight open"
        );
    }

    #[test]
    fn open_background_task_success_transitions_connection_state_to_connected() {
        let fn_source = open_background_task_source();

        assert!(sym(&fn_source, "Ok(outcome) => {"));
        assert!(sym(
            &fn_source,
            "cx.set_global(EditorProjectConnectionState::connected());"
        ));

        let connected_pos = sym_pos(
            &fn_source,
            "cx.set_global(EditorProjectConnectionState::connected());",
        )
        .expect("success arm must set EditorProjectConnectionState::connected()");
        let aggregate_pos = sym_pos(
            &fn_source,
            "install_verified_attached_session(cx, attach_session)",
        )
        .expect("success arm must install the verified controller aggregate");
        assert!(
            aggregate_pos < connected_pos,
            "the validated Installing aggregate must be published before panels can observe \
             Connected"
        );
        assert!(
            !sym(&fn_source, "cx.defer("),
            "controller installation must not be deferred until after Connected"
        );
    }

    #[test]
    fn verified_session_installer_publishes_addable_components_and_one_aggregate() {
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("project_workflow.rs"),
        )
        .expect("read project_workflow.rs");
        let start = source
            .find("pub(crate) fn install_verified_attached_session(")
            .expect("find verified session installer");
        let end = source[start..]
            .find("\nfn install_project_workflow_attached_session_globals(")
            .map(|offset| start + offset)
            .expect("find verified session installer end");
        let installer = &source[start..end];
        assert!(sym(
            installer,
            "install_attached_controllers(cx, &controller_session)"
        ));
        let globals_start = end;
        let globals_end = source[globals_start..]
            .find("\nfn publish_project_workflow_outcome(")
            .map(|offset| globals_start + offset)
            .expect("find attached session globals installer end");
        let globals = &source[globals_start..globals_end];
        assert!(sym(
            globals,
            "cx.set_global(EditorAddableAuthoredComponents::new("
        ));
        assert!(sym(globals, "addable_reflected_component_data"));
    }

    #[test]
    fn open_background_task_failure_transitions_connection_state_to_failed() {
        // Every failure inside `open_project_workflow_editor_session_with_daemon_endpoint`
        // (service start via `ensure_project_session_services_with_progress`, attach via
        // `attach_to_session_via_daemon`, or the daemon RPC connect/register
        // calls themselves) propagates through `?` into the single `Result`
        // matched here, so asserting on this one `Err` arm covers "failure at
        // service start" and "failure at attach" alike.
        let fn_source = open_background_task_source();

        assert!(sym(&fn_source, "Err(err) => {"));
        let fn_symbols = az_architecture_guard::symbol_skeleton(&fn_source);
        let err_arm_start = fn_symbols
            .find(&az_architecture_guard::symbol_skeleton("Err(err) => {"))
            .expect("background task must match the daemon-endpoint open result");
        let err_arm_source = &fn_symbols[err_arm_start..];

        let failed_pos = err_arm_source
            .find(&az_architecture_guard::symbol_skeleton(
                "cx.set_global(EditorProjectConnectionState::failed(err.to_string()));",
            ))
            .expect(
                "failure arm must set EditorProjectConnectionState::failed(..) with the error \
                 reason; without this, the connection state is stuck on Connecting forever",
            );
        let publish_error_pos = err_arm_source
            .find(&az_architecture_guard::symbol_skeleton(
                "publish_project_workflow_error(",
            ))
            .expect("failure arm must publish the project workflow error");
        assert!(
            failed_pos < publish_error_pos,
            "connection state must move to Failed before publishing the workflow status error"
        );
    }

    #[test]
    fn initializes_existing_project_through_shared_scaffold() {
        let temp = tempfile::tempdir().unwrap();

        let outcome = initialize_project(&ProjectWorkflowInitRequest {
            path: temp.path().to_path_buf(),
            name: Some("existing-game".to_string()),
            lore_url: None,
        })
        .unwrap();

        assert_eq!(outcome.operation, ui::Operation::InitializeProject);
        assert!(
            outcome
                .next_steps
                .iter()
                .any(|step| step.kind == ui::NextStepKind::CreateMainSession
                    && step.command.contains("session create main"))
        );
        assert!(temp.path().join("azoth.toml").exists());
        // The shared scaffold produces one layout: a primary-gem project whose
        // runtime, authoring, and builder role packages live under
        // `gems/<slug>`. The retired `crates/game` cluster is not a fallback.
        let manifest = az_project_scaffold::load_project_summary(temp.path()).unwrap();
        assert_eq!(manifest.name, "existing-game");
        assert!(temp.path().join("gems/existing-game/gem.toml").is_file());
        for role in ["runtime", "authoring", "builders"] {
            assert!(
                temp.path()
                    .join("gems/existing-game")
                    .join(role)
                    .join("Cargo.toml")
                    .is_file(),
                "initialized project must own a `{role}` role package"
            );
        }
        assert!(temp.path().join("Cargo.toml").is_file());
        assert!(temp.path().join("azoth.lock").is_file());
        assert!(!temp.path().join("crates/game").exists());
    }

    #[test]
    fn project_workflow_step_kind_mapping_preserves_attach_intents() {
        assert_eq!(
            project_workflow_next_step_kind_to_ui(
                az_project_scaffold::new::ProjectWorkflowNextStepKind::CreateMainSession
            ),
            ui::NextStepKind::CreateMainSession
        );
        assert_eq!(
            project_workflow_next_step_kind_to_ui(
                az_project_scaffold::new::ProjectWorkflowNextStepKind::OpenEditorSession
            ),
            ui::NextStepKind::OpenEditorSession
        );
    }
}
