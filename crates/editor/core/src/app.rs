//! Main editor application
//!
//! GPUI-based application window that hosts all editor panels.

use crate::EditorAttachSession;
use crate::asset_processor::install_asset_browser_action_handlers;
use crate::error::{EditorError, EditorResult};
use crate::gpu::init_editor_gpu;
use crate::graph_ui::install_graph_action_handlers;
use crate::mannequin_animation::install_mannequin_animation_action_handlers;
use crate::project_build::install_project_build_action_handlers;
use crate::project_workflow::{
    ProjectWorkflowEditorSessionOpenRequest, install_project_workflow_action_handlers,
    install_verified_attached_session, open_project_workflow_editor_session_from_action,
    spawn_open_project_workflow_editor_session_with_daemon_endpoint,
};
use crate::runtime_host::install_runtime_action_handlers;
use crate::scene_tools::{install_scene_tool_action_handlers, install_scene_tool_controller};
use crate::session_supervisor::install_session_status_action_handlers;
use crate::settings::load_settings_store;
use az_editor_ui::actions;
use az_editor_ui::panels::project_workflow;
use az_editor_ui::panels::{
    ConsoleState, EditorBlendSpacePreview, EditorMannequinPreview, EditorViewportRenderStatus,
    LogLevel, OutputLogState,
};
use az_editor_ui::viewport_texture::SharedViewportTexture;
use az_proto_core::Endpoint;
use az_proto_project::ProjectInventoryReport;
use gpui::{
    AnyView, AppContext, Context, Entity, Global, Hsla, IntoElement, ParentElement, Render, Styled,
    Window, WindowHandle, WindowOptions, div, px,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{Icon, IconName, Root, Sizable as _, TitleBar};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use tracing::{error, info, warn};

mod dc_icons;
mod project_manager;
mod project_template_model;

// Adopted Aether views. They started as htmlswap reference output, but the
// editor now owns them as stable runtime code. Future design regenerations are
// used as area-level reference material, not whole-file replacements.
#[allow(dead_code, unused, clippy::all, clippy::pedantic, clippy::nursery)]
mod aether_asset_processor_model;
#[allow(dead_code, unused, clippy::all, clippy::pedantic, clippy::nursery)]
mod aether_asset_processor_view;
#[allow(dead_code, unused, clippy::all, clippy::pedantic, clippy::nursery)]
mod aether_common;
#[allow(dead_code, unused, clippy::all, clippy::pedantic, clippy::nursery)]
mod aether_editor_model;
#[allow(dead_code, unused, clippy::all, clippy::pedantic, clippy::nursery)]
mod aether_editor_view;
#[allow(dead_code, unused, clippy::all, clippy::pedantic, clippy::nursery)]
mod aether_editor_view_render;
mod project_manager_view;
pub use aether_editor_view::AetherEditorView as EditorView;

use aether_asset_processor_view::AetherAssetProcessorView;
use project_manager::ProjectManagerView;
use project_template_model::{
    ProjectColorSpace, ProjectRenderPipeline, ProjectRendererBackend, ProjectTargetPlatform,
    ProjectTemplateKind, ProjectTemplateStep, ProjectTopologyChoice, default_project_location,
    template_enabled_gems, template_lore_url, template_project_name, template_project_path,
};

fn publish_gem_catalog(cx: &mut gpui::App) {
    match discover_engine_gems() {
        Ok(gems) => {
            info!(
                gem_count = gems.len(),
                "discovered engine gems for project manager"
            );
            cx.set_global(az_editor_ui::EditorGemCatalog::new(gems));
        }
        Err(error) => {
            warn!(%error, "failed to discover engine gems for project manager");
            cx.set_global(az_editor_ui::EditorGemCatalog::error(error));
        }
    }
}

pub(crate) fn gem_selection_from_project_inventory(
    inventory: &ProjectInventoryReport,
) -> az_editor_ui::EditorGemSelection {
    az_editor_ui::EditorGemSelection::new(
        inventory
            .gems
            .iter()
            .filter(|gem| gem.expected)
            .map(|gem| gem.id.clone())
            .collect(),
    )
}

fn discover_engine_gems() -> Result<Vec<az_editor_ui::EditorGemInfo>, String> {
    let summaries =
        az_project_scaffold::load_engine_gem_summaries().map_err(|error| error.to_string())?;
    let cargo_manifests = summaries
        .iter()
        .map(|gem| (gem.id.clone(), gem_cargo_manifest(&gem.root)))
        .collect::<BTreeMap<_, _>>();
    let gem_manifests = summaries
        .iter()
        .map(|gem| {
            az_project_scaffold::load_gem_manifest(&gem.root)
                .map(|manifest| (gem.id.clone(), manifest))
                .map_err(|error| error.to_string())
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let gem_name_by_id = summaries
        .iter()
        .map(|gem| (gem.id.clone(), gem.name.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut gems = summaries
        .into_iter()
        .map(|gem| {
            let cargo_manifest = cargo_manifests.get(&gem.id).and_then(Option::as_ref);
            let category = gem_category(&gem.id, &gem.name).to_string();
            let dependencies = gem_manifests
                .get(&gem.id)
                .map(|manifest| gem_manifest_dependencies(manifest, &gem_name_by_id))
                .unwrap_or_default();
            let deprecation = gem_manifests
                .get(&gem.id)
                .and_then(|manifest| gem_manifest_deprecation(manifest, &gem_name_by_id));
            az_editor_ui::EditorGemInfo {
                id: gem.id,
                name: gem.name,
                version: gem.version,
                description: cargo_manifest
                    .and_then(gem_cargo_description)
                    .unwrap_or_default(),
                category,
                dependencies,
                deprecation,
            }
        })
        .collect::<Vec<_>>();
    gems.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(gems)
}

fn gem_cargo_manifest(root: &Path) -> Option<toml::Value> {
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    toml::from_str::<toml::Value>(&manifest).ok()
}

fn gem_cargo_description(manifest: &toml::Value) -> Option<String> {
    manifest
        .get("package")?
        .get("description")?
        .as_str()
        .map(str::to_string)
}

fn gem_manifest_dependencies(
    manifest: &az_project_scaffold::GemManifest,
    gem_name_by_id: &BTreeMap<String, String>,
) -> Vec<az_editor_ui::EditorGemDependencyInfo> {
    let mut resolved = manifest
        .dependencies
        .iter()
        .map(|dependency| az_editor_ui::EditorGemDependencyInfo {
            id: dependency.id.clone(),
            name: gem_name_by_id
                .get(&dependency.id)
                .cloned()
                .unwrap_or_else(|| dependency.id.clone()),
        })
        .collect::<Vec<_>>();
    resolved.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.id.cmp(&right.id))
    });
    resolved.dedup_by(|left, right| left.id == right.id);
    resolved
}

fn gem_manifest_deprecation(
    manifest: &az_project_scaffold::GemManifest,
    gem_name_by_id: &BTreeMap<String, String>,
) -> Option<az_editor_ui::EditorGemDeprecationInfo> {
    let deprecation = manifest.gem.deprecation.as_ref()?;
    Some(az_editor_ui::EditorGemDeprecationInfo {
        message: deprecation.message.clone(),
        since: deprecation.since.clone(),
        replacement: deprecation.replacement.as_ref().map(|replacement| {
            az_editor_ui::EditorGemReplacementInfo {
                id: replacement.id.clone(),
                name: gem_name_by_id
                    .get(&replacement.id)
                    .cloned()
                    .unwrap_or_else(|| replacement.id.clone()),
                version: replacement.version.clone(),
            }
        }),
    })
}

fn gem_category(id: &str, name: &str) -> &'static str {
    let key = format!("{id} {name}").to_ascii_lowercase();
    if key.contains("core")
        || key.contains("app")
        || key.contains("framework")
        || key.contains("bootstrap")
    {
        "System"
    } else if key.contains("gamedata") || key.contains("data") {
        "Data"
    } else if key.contains("ui") || key.contains("lyshine") {
        "UI"
    } else if key.contains("render")
        || key.contains("terrain")
        || key.contains("texture")
        || key.contains("material")
        || key.contains("shader")
    {
        "Rendering"
    } else if key.contains("audio") || key.contains("sound") {
        "Audio"
    } else if key.contains("physics") || key.contains("cloth") {
        "Physics"
    } else if key.contains("world") || key.contains("level") || key.contains("scene") {
        "World"
    } else if key.contains("steam")
        || key.contains("eos")
        || key.contains("platform")
        || key.contains("xr")
        || key.contains("openxr")
    {
        "Platform"
    } else if key.contains("camera")
        || key.contains("input")
        || key.contains("gameplay")
        || key.contains("animation")
    {
        "Gameplay"
    } else {
        "Other"
    }
}

fn editor_window_title(session: Option<&EditorAttachSession>) -> String {
    session.map_or_else(
        || "Aether".to_owned(),
        |session| format!("Aether — {}", attached_project_window_name(session)),
    )
}

fn asset_processor_window_title(session: Option<&EditorAttachSession>) -> String {
    session.map_or_else(
        || "Aether Asset Processor".to_owned(),
        |session| {
            format!(
                "Aether Asset Processor — {}",
                attached_project_window_name(session)
            )
        },
    )
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

fn chrome_icon(icon: IconName, size: f32, color: impl Into<Hsla>) -> impl IntoElement {
    Icon::new(icon).with_size(px(size)).text_color(color)
}

fn chrome_tooltip(
    label: &'static str,
) -> impl Fn(&mut Window, &mut gpui::App) -> AnyView + Clone + 'static {
    move |window, cx| Tooltip::new(label).build(window, cx)
}

#[derive(Default)]
struct AssetProcessorWindowState {
    handle: Option<WindowHandle<Root>>,
}

impl Global for AssetProcessorWindowState {}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OpenAssetProcessorAfterAttach;

impl Global for OpenAssetProcessorAfterAttach {}

pub(crate) fn request_open_asset_processor_after_attach(cx: &mut gpui::App) {
    cx.set_global(OpenAssetProcessorAfterAttach);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EditorShellMode {
    Workspace,
    AssetProcessorOnly,
}

#[derive(Debug)]
struct PendingProjectWorkflowOpen {
    request: ProjectWorkflowEditorSessionOpenRequest,
    daemon_endpoint: Endpoint,
}

fn install_asset_processor_window_action_handler(cx: &mut gpui::App) {
    cx.on_action(|_: &actions::OpenAssetProcessor, cx| {
        if cx.try_global::<EditorAttachSession>().is_some() {
            if let Err(error) = open_or_focus_asset_processor_window(cx) {
                error!(%error, "failed to open asset processor window");
            }
            return;
        }

        let Some(action) = project_manager::default_recent_project_open_action() else {
            cx.default_global::<ConsoleState>().log_from_source(
                LogLevel::Warn,
                "project-manager",
                "Open a project before launching Asset Processor.",
            );
            cx.refresh_windows();
            return;
        };

        request_open_asset_processor_after_attach(cx);
        if !open_project_workflow_editor_session_from_action(&action, cx) {
            cx.remove_global::<OpenAssetProcessorAfterAttach>();
        }
    });
}

pub(crate) fn open_or_focus_asset_processor_window(cx: &mut gpui::App) -> EditorResult<()> {
    if cx.try_global::<EditorAttachSession>().is_none() {
        return Err(EditorError::MissingAttachedSession {
            operation: "open asset processor window",
        });
    }

    if let Some(handle) = cx
        .try_global::<AssetProcessorWindowState>()
        .and_then(|state| state.handle)
        && handle
            .update(cx, |_, window, _| {
                window.activate_window();
            })
            .is_ok()
    {
        return Ok(());
    }

    let handle = open_asset_processor_window(cx)?;
    cx.global_mut::<AssetProcessorWindowState>().handle = Some(handle);
    Ok(())
}

fn open_asset_processor_window(cx: &mut gpui::App) -> EditorResult<WindowHandle<Root>> {
    let title = asset_processor_window_title(cx.try_global::<EditorAttachSession>());
    let mut titlebar = TitleBar::title_bar_options();
    titlebar.title = Some(title.clone().into());
    let window_options = WindowOptions {
        titlebar: Some(titlebar),
        ..Default::default()
    };

    let handle = cx
        .open_window(window_options, move |window, cx| {
            window.set_window_title(&title);
            let view = cx.new(|cx| AetherAssetProcessorView::new(window, cx));
            cx.new(|cx| Root::new(view, window, cx))
        })
        .map_err(|error| EditorError::Window(error.to_string()))?;
    let _ = handle.update(cx, |_, window, _| {
        window.activate_window();
    });
    Ok(handle)
}

/// Main editor application
///
/// Entry point for running the `AZoth` Editor as a standalone application.
#[derive(Default, Debug)]
pub struct EditorApp {}

impl EditorApp {
    /// Create a new editor application
    ///
    /// # Errors
    ///
    /// Infallible today: [`Self::run`] installs the window on GPUI's own task
    /// and returns before it resolves, so a window-open failure is reported
    /// through the editor log rather than returned here. The `Result` is kept
    /// because the other constructors on this type resolve a session first.
    pub fn new() -> EditorResult<Self> {
        Ok(Self::run(EditorShellMode::Workspace, None, None))
    }

    /// Create a new editor application attached to a workspace session.
    ///
    /// # Errors
    ///
    /// Infallible today, for the same reason as [`Self::new`]: the session is
    /// already resolved by the caller, and window installation is deferred to
    /// a GPUI task.
    pub fn attach(session: EditorAttachSession) -> EditorResult<Self> {
        Ok(Self::run(EditorShellMode::Workspace, Some(session), None))
    }

    /// Resolve and verify a session, then create the editor application.
    ///
    /// # Errors
    ///
    /// Returns any error [`crate::attach_to_session`] returns — no active or
    /// an ambiguous session for the project, a session that is not active, a
    /// project record that does not match `project_root`, or a manifest
    /// missing the project services the editor attaches to.
    pub fn open_session(
        project_root: impl AsRef<std::path::Path>,
        requested_session: Option<&str>,
    ) -> EditorResult<Self> {
        let session = crate::attach_to_session(project_root, requested_session)?;
        Self::attach(session)
    }

    /// Resolve and verify a session through azd, then create the editor application.
    ///
    /// # Errors
    ///
    /// Returns any error [`crate::attach_to_session_via_daemon`] returns — a
    /// failed daemon RPC, no active or an ambiguous session, an inactive
    /// session, a project record that does not match `project_root`, or a
    /// manifest missing the project services the editor attaches to.
    pub fn open_session_via_daemon(
        project_root: impl AsRef<std::path::Path>,
        requested_session: Option<&str>,
        daemon_endpoint: Endpoint,
    ) -> EditorResult<Self> {
        let session =
            crate::attach_to_session_via_daemon(project_root, requested_session, daemon_endpoint)?;
        Self::attach(session)
    }

    /// Ensure a project workflow session through azd, start its project
    /// services, then create the attached editor application.
    ///
    /// # Errors
    ///
    /// Infallible today: the workflow is handed to the shell as a pending open
    /// and runs on GPUI's task queue, so a failure to ensure the session or
    /// start its services surfaces in the project-open progress UI rather than
    /// being returned here.
    pub fn open_project_session_via_daemon(
        project_root: impl AsRef<std::path::Path>,
        session_name: &str,
        daemon_endpoint: Endpoint,
    ) -> EditorResult<Self> {
        Ok(Self::run(
            EditorShellMode::Workspace,
            None,
            Some(PendingProjectWorkflowOpen {
                request: ProjectWorkflowEditorSessionOpenRequest {
                    project_root: project_root.as_ref().to_path_buf(),
                    session_name: session_name.to_string(),
                },
                daemon_endpoint,
            }),
        ))
    }

    /// Ensure a project workflow session through azd, then show only the live
    /// Asset Processor window for that session.
    ///
    /// # Errors
    ///
    /// Infallible today, for the same reason as
    /// [`Self::open_project_session_via_daemon`]: the workflow runs as a
    /// pending open on GPUI's task queue and reports failure through the UI.
    pub fn open_asset_processor_session_via_daemon(
        project_root: impl AsRef<std::path::Path>,
        session_name: &str,
        daemon_endpoint: Endpoint,
    ) -> EditorResult<Self> {
        Ok(Self::run(
            EditorShellMode::AssetProcessorOnly,
            None,
            Some(PendingProjectWorkflowOpen {
                request: ProjectWorkflowEditorSessionOpenRequest {
                    project_root: project_root.as_ref().to_path_buf(),
                    session_name: session_name.to_string(),
                },
                daemon_endpoint,
            }),
        ))
    }

    fn run(
        shell_mode: EditorShellMode,
        attach_session: Option<EditorAttachSession>,
        pending_project_open: Option<PendingProjectWorkflowOpen>,
    ) -> Self {
        let ui_state_project_root = attach_session
            .as_ref()
            .map(|session| session.project_root.as_path())
            .or_else(|| {
                pending_project_open
                    .as_ref()
                    .map(|pending| pending.request.project_root.as_path())
            });
        let mut ui_state_bootstrap =
            crate::ui_state_persistence::load_bootstrap(ui_state_project_root);
        #[cfg(target_os = "windows")]
        let viewport_product_root =
            windows_viewport_product_root(attach_session.as_ref(), pending_project_open.as_ref());
        #[cfg(target_os = "windows")]
        let viewport_render_bootstrap = (shell_mode != EditorShellMode::AssetProcessorOnly)
            .then(crate::editor_render::EditorRenderBootstrap::initialize);
        gpui_platform::application()
            .with_assets(gpui_component_assets::Assets)
            .run(move |cx| {
                let attached_project = attach_session.is_some();
                let pending_project_root = pending_project_open
                    .as_ref()
                    .map(|pending| pending.request.project_root.as_path());
                info!(
                    attached_project,
                    pending_project_open = pending_project_open.is_some(),
                    ?shell_mode,
                    "editor app starting"
                );

                gpui_component::init(cx);

                if let Some(bootstrap) = ui_state_bootstrap.take() {
                    crate::ui_state_persistence::install(cx, bootstrap);
                }

                install_editor_action_handlers(cx);

                // Initialize settings (global + optional attached-session override) as a GPUI global
                let project_root = attach_session
                    .as_ref()
                    .map(|session| session.project_root.as_path())
                    .or(pending_project_root);
                install_editor_globals(
                    cx,
                    project_root,
                    attached_project || pending_project_open.is_some(),
                );
                install_editor_chrome(cx);

                let initial_window_title = editor_window_title(attach_session.as_ref());
                let initial_window_bounds = crate::ui_state_persistence::restored_window_bounds(cx)
                    .unwrap_or_else(|| crate::ui_state_persistence::default_window_bounds(cx));

                start_pending_project_work(cx, shell_mode, attach_session, pending_project_open);

                // (Console global init/logs removed; panels now managed via dock registry)
                cx.spawn(async move |cx| {
                    if shell_mode == EditorShellMode::AssetProcessorOnly {
                        return Ok::<_, EditorError>(());
                    }

                    let window_options =
                        editor_main_window_options(&initial_window_title, initial_window_bounds);

                    // The app shell is the sole root view; it slides in the
                    // editor workspace (attached/connecting) or the project
                    // manager based on session state.
                    #[cfg(target_os = "windows")]
                    let viewport_product_root = viewport_product_root.clone();
                    #[cfg(target_os = "windows")]
                    let viewport_render_bootstrap = viewport_render_bootstrap
                        .expect("workspace mode must initialize viewport D3D12 resources");
                    cx.open_window(window_options, move |window, cx| {
                        window.set_window_title(&initial_window_title);
                        #[cfg(target_os = "windows")]
                        install_window_viewport_host(
                            window,
                            cx,
                            viewport_product_root,
                            viewport_render_bootstrap,
                        );
                        let view = cx.new(|cx| AppShell::new(window, cx));
                        cx.new(|cx| Root::new(view, window, cx))
                    })
                    .map(|_| ())
                    .map_err(|e| EditorError::Window(e.to_string()))?;

                    Ok::<_, EditorError>(())
                })
                .detach();
            });
        Self {}
    }
}

/// Window options for the main editor window. The editor workspace is
/// full-viewport chrome (design: Aether Editor fills the screen), so the window
/// opens maximized and `bounds` is only the restore size.
fn editor_main_window_options(title: &str, bounds: gpui::WindowBounds) -> WindowOptions {
    let mut titlebar = TitleBar::title_bar_options();
    titlebar.title = Some(title.to_owned().into());
    WindowOptions {
        titlebar: Some(titlebar),
        window_bounds: Some(bounds),
        ..Default::default()
    }
}

/// Resolves the platform product-cache directory the viewport renderer reads
/// from: the attached session names it directly, a pending open reads it out of
/// the project manifest.
#[cfg(target_os = "windows")]
fn windows_viewport_product_root(
    attach_session: Option<&EditorAttachSession>,
    pending_project_open: Option<&PendingProjectWorkflowOpen>,
) -> Option<std::path::PathBuf> {
    if let Some(session) = attach_session {
        return Some(az_filesystem::project_default_platform_product_cache_dir(
            &session.project_id,
            &session.project_root,
        ));
    }
    let project_root = pending_project_open?.request.project_root.as_path();
    let manifest = std::fs::read_to_string(project_root.join("azoth.toml")).ok()?;
    let manifest = toml::from_str::<toml::Value>(&manifest).ok()?;
    let project = manifest.get("project")?;
    let project_name = project
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            project
                .get("id")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })?;
    Some(az_filesystem::project_default_platform_product_cache_dir(
        project_name,
        project_root,
    ))
}

/// Boots the D3D12 viewport producer against the new window's composition
/// visual and installs it as the editor's viewport host.
#[cfg(target_os = "windows")]
fn install_window_viewport_host(
    window: &Window,
    cx: &mut gpui::App,
    viewport_product_root: Option<std::path::PathBuf>,
    bootstrap: Result<
        crate::editor_render::EditorRenderBootstrap,
        crate::editor_render::EditorRenderInitError,
    >,
) {
    let producer = bootstrap
        .map_err(crate::viewport_host::ViewportBootError::Renderer)
        .and_then(|bootstrap| {
            gpui_windows::viewport_visual_slot(window.window_handle().window_id().as_u64())
                .ok_or(crate::viewport_host::ViewportBootError::MissingCompositionVisual)
                .and_then(|slot| {
                    crate::viewport_host::boot_viewport_renderer(
                        viewport_product_root,
                        slot,
                        bootstrap,
                    )
                })
        });
    crate::viewport_host::install_viewport_host(cx, producer);
}

/// Registers every editor-shell action handler exactly once, at startup.
fn install_editor_action_handlers(cx: &mut gpui::App) {
    actions::register_actions(cx);
    cx.set_global(AssetProcessorWindowState::default());
    install_asset_processor_window_action_handler(cx);
    crate::authored_selection::install_reflected_selection_action_handlers(cx);
    crate::console::install_console_action_handlers(cx);
    install_project_workflow_action_handlers(cx);
    install_asset_browser_action_handlers(cx);
    install_graph_action_handlers(cx);
    crate::mode_projection::install_mode_projections(cx)
        .expect("editor mode projection registrations must be unique");
    install_runtime_action_handlers(cx);
    install_session_status_action_handlers(cx);
    install_scene_tool_action_handlers(cx);
    install_project_build_action_handlers(cx);
    install_mannequin_animation_action_handlers(cx);
    crate::sequencer::install_sequencer_action_handlers(cx);
    crate::controller_set::install_controller_retry_action_handler(cx);
}

/// Publishes the globals the shell reads before any window exists: the viewport
/// texture slot, project-scoped previews and settings, and the log sinks.
fn install_editor_globals(cx: &mut gpui::App, project_root: Option<&Path>, has_project: bool) {
    // Initialize a neutral viewport texture slot for runtime-host viewport
    // streams (play-in-standalone). The editor's scene viewport is the
    // in-process renderer installed alongside the window.
    init_editor_gpu(cx);
    cx.set_global(SharedViewportTexture::new(1920, 1080));
    cx.set_global(EditorViewportRenderStatus::waiting());

    let mannequin_preview = project_root.map_or_else(
        EditorMannequinPreview::empty,
        EditorMannequinPreview::default_for_project_root,
    );
    let blend_space_preview =
        project_root.map_or_else(EditorBlendSpacePreview::empty, |project_root| {
            EditorBlendSpacePreview::with_project_asset_root(project_root.join("assets"))
        });
    cx.set_global(mannequin_preview);
    cx.set_global(blend_space_preview);
    cx.set_global(load_settings_store(project_root));

    // Install tracing layer and start async forwarder to Console.
    cx.set_global(ConsoleState::default());
    cx.set_global(OutputLogState::default());
    install_scene_tool_controller(cx);
    cx.set_global(if has_project {
        project_workflow::Status::default()
    } else {
        project_workflow::Status::new_project_requested()
    });
    crate::logging::hook();
    crate::logging::run(cx);
}

/// Registers the menus, panels, keymaps, theme, and gem catalog the editor
/// chrome renders from.
fn install_editor_chrome(cx: &mut gpui::App) {
    // Register application menus (new menu builder). Set both the native menu
    // (cmd-palette / macOS bar) and the gpui-component in-window menu bar
    // source so the AZoth menu/title bar renders working File/Edit/View/…
    // dropdowns on Windows.
    cx.set_menus(az_editor_ui::menu::build_app_menus());
    let owned_menus = az_editor_ui::menu::build_app_menus()
        .into_iter()
        .map(gpui::Menu::owned)
        .collect();
    gpui_component::GlobalState::global_mut(cx).set_app_menus(owned_menus);

    // Register panels to gpui_component registry
    az_editor_ui::panels::register_default_panels(cx);

    // Bind default keymaps/shortcuts
    crate::keymaps::bind_default_keymap(cx);

    // Initialize theme system
    if let Err(e) = crate::theme::init(cx) {
        error!("Failed to initialize theme system: {}", e);
    }
    publish_gem_catalog(cx);
}

/// Starts the pending open workflow and installs a session the caller already
/// verified, so the first frame renders against real project state.
fn start_pending_project_work(
    cx: &mut gpui::App,
    shell_mode: EditorShellMode,
    attach_session: Option<EditorAttachSession>,
    pending_project_open: Option<PendingProjectWorkflowOpen>,
) {
    if shell_mode == EditorShellMode::AssetProcessorOnly && pending_project_open.is_some() {
        request_open_asset_processor_after_attach(cx);
    }

    if let Some(pending) = pending_project_open {
        let project_root_label = pending.request.project_root.display().to_string();
        cx.set_global(project_workflow::Status::running(
            project_workflow::Operation::OpenEditorSession,
            project_root_label.clone(),
        ));
        cx.set_global(az_editor_ui::panels::EditorProjectConnectionState::connecting());
        spawn_open_project_workflow_editor_session_with_daemon_endpoint(
            cx,
            pending.request,
            Some(pending.daemon_endpoint),
            project_root_label,
            Vec::new(),
        );
    }

    if let Some(session) = attach_session {
        if let Err(error) = install_verified_attached_session(cx, session) {
            error!(%error, "failed to install initial verified attached editor session");
        }
        if shell_mode == EditorShellMode::AssetProcessorOnly
            && let Err(error) = open_or_focus_asset_processor_window(cx)
        {
            error!(%error, "failed to open asset processor window during startup");
        }
    }
}

#[derive(Debug)]
/// Top-level app shell: the sole root view.
///
/// It slides in the active view — the loaded editor workspace once a project is
/// attaching/attached, otherwise the project manager. Both child views are
/// created lazily. Adding a top-level view is a field plus a render arm.
#[derive(Default)]
pub struct AppShell {
    manager: Option<Entity<ProjectManagerView>>,
    workspace: Option<Entity<EditorView>>,
    last_logged_shell_state: Option<ShellState>,
    ui_state_subscriptions: Vec<gpui::Subscription>,
}

impl AppShell {
    #[must_use]
    pub fn new(window: &mut Window, cx: &mut Context<'_, Self>) -> Self {
        let mut this = Self {
            manager: None,
            workspace: None,
            last_logged_shell_state: None,
            ui_state_subscriptions: Vec::new(),
        };
        if cx.has_global::<crate::ui_state_persistence::UiStatePersistence>() {
            this.ui_state_subscriptions
                .push(cx.observe_window_bounds(window, |_, window, cx| {
                    crate::ui_state_persistence::capture_window(window, cx);
                }));
        }
        this
    }

    fn manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) -> Entity<ProjectManagerView> {
        if let Some(manager) = &self.manager {
            return manager.clone();
        }
        let manager = cx.new(|cx| ProjectManagerView::new(window, cx));
        self.manager = Some(manager.clone());
        manager
    }

    fn workspace(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> Entity<EditorView> {
        if let Some(workspace) = &self.workspace {
            return workspace.clone();
        }
        info!("app shell creating loaded editor workspace");
        let workspace = cx.new(|cx| EditorView::new(window, cx));
        self.workspace = Some(workspace.clone());
        workspace
    }

    fn log_shell_state(&mut self, state: ShellState, status: Option<&project_workflow::Status>) {
        if self.last_logged_shell_state == Some(state) {
            return;
        }
        let mode = status.map(|status| status.mode);
        let phase = status.map(|status| status.phase);
        let operation = status.and_then(|status| status.operation);
        info!(?state, ?mode, ?phase, ?operation, "app shell state changed");
        self.last_logged_shell_state = Some(state);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellState {
    Launcher,
    LoadedWorkspace,
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) -> impl IntoElement {
        let title = editor_window_title(cx.try_global::<EditorAttachSession>());
        window.set_window_title(&title);

        // Transition into the loaded-project workspace as soon as a project open
        // begins connecting (Connecting), not only once fully attached. The
        // workspace renders a progress banner and gates project-host panels while
        // connecting; it flips to live data on attach.
        //
        // Stay in the workspace on a failed open (Failed) too, rather than
        // silently bouncing back to the launcher: the gated panels render the
        // failure reason in place and offer a "Back to Project Launcher"
        // affordance (`BackToProjectLauncher`) that the user drives explicitly.
        let attached = cx.try_global::<EditorAttachSession>().is_some();
        let connecting = az_editor_ui::panels::editor_project_host_connecting(cx);
        let failed = az_editor_ui::panels::editor_project_host_failed_reason(cx).is_some();
        if attached || connecting || failed {
            self.log_shell_state(ShellState::LoadedWorkspace, None);
            return div()
                .size_full()
                .relative()
                .child(self.workspace(window, cx))
                .into_any_element();
        }

        let status = cx
            .try_global::<project_workflow::Status>()
            .cloned()
            .unwrap_or_default();
        self.log_shell_state(ShellState::Launcher, Some(&status));
        self.manager(window, cx).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::gem_selection_from_project_inventory;
    use az_proto_project::{
        ProjectInventoryGem, ProjectInventoryGemKind, ProjectInventoryLockState,
        ProjectInventoryLockStatus, ProjectInventoryRegistryCounts, ProjectInventoryReport,
    };

    #[test]
    fn attached_project_inventory_publishes_expected_gems_as_selection() {
        let inventory = ProjectInventoryReport {
            service_role: "project-host".to_owned(),
            lock_status: ProjectInventoryLockStatus {
                state: ProjectInventoryLockState::Fresh,
                path: "azoth.lock".to_owned(),
                diagnostic: String::new(),
            },
            gems: vec![
                inventory_gem("azoth.render", true, true),
                inventory_gem("azoth.experimental", false, true),
            ],
            registry: ProjectInventoryRegistryCounts {
                build_rules: 0,
                node_types: 0,
                graph_types: 0,
            },
            diagnostics: Vec::new(),
            degraded: false,
        };

        let selection = gem_selection_from_project_inventory(&inventory);
        assert_eq!(selection.enabled, vec!["azoth.render"]);
    }

    fn inventory_gem(id: &str, expected: bool, active: bool) -> ProjectInventoryGem {
        ProjectInventoryGem {
            id: id.to_owned(),
            expected_package: id.replace('.', "_"),
            name: id.to_owned(),
            version: "0.1.0".to_owned(),
            kind: ProjectInventoryGemKind::Engine,
            expected,
            active,
            capabilities: Vec::new(),
        }
    }
}
