//! `AZoth` Editor.
//!
//! GPUI-based universal editor for the `AZoth` engine. The editor is a
//! standalone application that attaches to project workspace sessions and talks
//! to project-owned services over Cap'n Proto IPC. Project and gem native code
//! is loaded by project-host, asset-processor, and runtime-host processes, not
//! by the editor UI process.
//!
//! # Usage
//!
//! ```rust,no_run
//! use az_editor::EditorApp;
//!
//! fn main() -> az_editor::EditorResult<()> {
//!     let _editor = EditorApp::new()?;
//!     Ok(())
//! }
//! ```

pub use az_editor_ui::actions;
pub mod app;
pub mod asset_processor;
pub mod attach;
pub mod authored_edit;
pub mod authored_selection;
// The standalone binary's argument surface. Private: the binary is the only
// caller, and it reaches it through `startup`.
mod cli;
pub mod console;
mod controller_set;
pub mod daemon;
pub mod daemon_bootstrap;
#[cfg(target_os = "windows")]
pub mod editor_render;
pub mod error;
pub mod game_data_catalog;
pub mod gpu;
pub mod graph_ui;
mod mannequin_animation;
mod materials_ui;
mod mode_projection;
pub mod perf;
pub mod sequencer;
pub use az_editor_ui::panels;

pub mod keymaps;
pub mod logging;
pub use az_editor_ui::menu;
pub mod project_build;
pub mod project_host;
mod project_manager_preferences;
pub mod project_open_progress;
pub mod project_workflow;
pub mod recent_projects;
pub mod recovery;
mod rpc_runtime;
pub mod runtime_host;
pub mod scene_tools;
mod scripting_ui;
mod service_descriptor;
pub mod session_supervisor;
pub mod settings;
pub mod source_navigation;
pub mod startup;
mod theme;
mod ui_state_persistence;
#[cfg(target_os = "windows")]
mod viewport_drag;
#[cfg(target_os = "windows")]
pub mod viewport_host;
#[cfg(target_os = "windows")]
mod viewport_resize;
mod watch_handle;
pub mod workspace;

// Re-exports
pub use app::EditorApp;
pub use asset_processor::{
    ASSET_BROWSER_PAGE_SIZE, AssetProcessorClient, EditorAssetBrowserController,
    create_asset_source_file, inspect_job, install_asset_browser_action_handlers,
    job_inspection_console_lines, job_inspection_to_ui, refresh_asset_browser_status,
    refresh_asset_browser_status_if_available, workspace_entry_page_to_ui,
};
pub use attach::{EditorAttachSession, attach_to_session, attach_to_session_via_daemon};
pub use authored_edit::{
    ReflectedHistoryDirection, ReflectedPrefabEdit, ReflectedPrefabEditSession,
};
pub use authored_selection::{
    EditorReflectedSelectionController, EditorReflectedSelectionState,
    ReflectedComponentInspection, ReflectedEntityInspection, ReflectedPrefabSelection,
    ReflectedSelectionError, install_reflected_selection_action_handlers,
    project_reflected_selection, publish_reflected_inspection,
};
pub use az_editor_inspector::{
    ReflectedAddComponent, ReflectedEditBinding, ReflectedInspectionField,
    ReflectedInspectionModel, ReflectedValidationState, WidgetFamily,
};
pub use console::{
    ConsoleCommand, install_console_action_handlers, parse_console_command, submit_console_command,
};
pub use daemon::AzDaemonClient;
pub use daemon_bootstrap::ensure_daemon_endpoint_for_project;
#[cfg(target_os = "windows")]
pub use editor_render::EditorSceneObject;
pub use editor_render::{EditorRenderApp, EditorRenderInitError};
pub use error::{EditorError, EditorResult};
pub use game_data_catalog::{
    EditorGameDataCatalog, EditorGameDataCatalogController, refresh_game_data_catalog,
};
pub use gpu::{
    EditorGpuAdapterReport, EditorGpuLaunchConfig, EditorGpuLaunchError, EditorGpuLaunchStatus,
    EditorGpuReadyState, EditorGpuState, EditorViewportGpuTexture, ViewportPickHit, ViewportScene,
    editor_gpu_device_descriptor, editor_gpu_instance, editor_gpu_request_adapter_options,
    editor_gpu_required_features, editor_gpu_required_limits, editor_gpu_status_from_ready,
    editor_viewport_texture_descriptor, init_editor_gpu, launch_editor_gpu,
    launch_editor_gpu_from_app, request_editor_gpu_adapter, request_editor_gpu_device,
    runtime_viewport_format_to_wgpu, runtime_viewport_texture_descriptor,
};
pub use graph_ui::{
    EditorGeneratedRustGraphAbiData, EditorGraphCompilerBackendData,
    EditorGraphCompilerBackendKindData, EditorGraphController, EditorGraphCreationCatalog,
    EditorGraphCreationError, EditorGraphCreationResult, EditorGraphRuntimeExecutionStrategyData,
    EditorGraphSourceWorkflowKindData, EditorGraphTypeCreationData, EditorGraphUiAdapter,
    EditorGraphUiAdapterError, EditorGraphUiResult, add_graph_node, auto_layout_current_graph,
    build_current_graph_document, connect_graph_ports, create_graph_document,
    graph_creation_catalog_from_graph_type_catalog, graph_document_id_from_creation_data,
    graph_document_projection_from_snapshot, install_graph_action_handlers, move_graph_node,
    publish_graph_projection_error, refresh_current_graph_build_status, refresh_graph_document,
    route_current_graph_connections, save_current_graph_document, set_graph_input_value,
};
pub use project_build::{
    EditorProjectBuildController, build_status_line_from_state,
    install_project_build_action_handlers, plan_project_build_from_action,
};
pub use project_host::{ProjectHostClient, ProjectRuntimeLaunchSnapshotContext};
pub use project_workflow::{
    EditorProjectWorkflowController, ProjectWorkflowCreateRequest,
    ProjectWorkflowEditorSessionOpenOutcome, ProjectWorkflowEditorSessionOpenRequest,
    ProjectWorkflowInitRequest, ProjectWorkflowOutcome, ProjectWorkflowServicePrepareOutcome,
    ProjectWorkflowServicePrepareRequest, ProjectWorkflowSessionOutcome,
    ProjectWorkflowSessionRequest, create_project, ensure_project_workflow_session,
    initialize_project, install_project_workflow_action_handlers,
    open_project_workflow_editor_session,
    open_project_workflow_editor_session_with_daemon_endpoint,
    prepare_project_workflow_session_services,
};
pub use recovery::{
    DEFAULT_AUTOSAVE_DEBOUNCE, DEFAULT_AUTOSAVE_INTERVAL, EditorRecoveryController,
    note_source_session_dirty, note_source_session_saved,
};
pub use runtime_host::{
    EDITOR_WORLD_RUNTIME_ID, EditorRuntimeController, RuntimeHostClient,
    install_runtime_action_handlers, launch_editor_world_from_action,
    refresh_editor_world_status_from_action, refresh_editor_world_viewport_frame_from_action,
    refresh_runtime_projection_catalog_from_action, runtime_status_to_ui,
    stop_editor_world_from_action,
};
pub use scene_tools::{install_scene_tool_action_handlers, install_scene_tool_controller};
pub use session_supervisor::{
    EditorSessionStatusController, ServiceLogQuery, SessionSupervisorClient,
    install_session_status_action_handlers, refresh_session_status, service_log_console_lines,
    service_log_output_events, service_log_output_lines, session_workspace_status_to_ui,
    stop_session_services, tail_session_service_log,
};
pub use source_navigation::{
    MaterialSourceKind, SourceNavigationIntent, SourceNavigationIntentKind,
    default_source_navigation_settings, join_script_source_path, material_source_kind,
    script_source_navigation_intent, source_navigation_intent,
};
#[cfg(target_os = "windows")]
pub use viewport_host::{
    EditorViewportHost, ViewportBootError, boot_viewport_renderer, install_viewport_host,
    viewport_scene_from_snapshots,
};

/// Editor version
pub const EDITOR_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const EDITOR_NAME: &str = "AZoth Editor";

#[cfg(test)]
mod architecture_tests {
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, GEM_INFRASTRUCTURE_DEPENDENCIES,
        PROJECT_OR_FORMAT_PATH_SEGMENTS, PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
        collect_production_rust_sources, forbidden_production_dependencies,
        production_source_without_cfg_test_modules, public_functions_have_cfg,
    };
    use toml::Value;

    const FORBIDDEN_PRODUCTION_DEPS: &[&str] = &[
        "az-asset",
        "az-asset-builder",
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        // Exact name only. The engine's contribution bundles
        // (`az-engine-types`/`-assets`/`-runtime`) are not this crate: they are
        // the manifested engine floor, and the viewport preview composes the
        // three its `runtime-host` role names instead of calling the
        // unattributed `engine_lowerings()` it used to (asset-contract ticket
        // 014, D6/F5). That is a narrower dependency than the floor it
        // replaced, not a wider one — the editor still links no gem and no
        // project code.
        "az-engine",
        // "az-framework" was ruled a sanctioned floor 2026-08-20 (registration
        // cutover): the render preview applies the engine's reflected types to
        // a local Bevy AppTypeRegistry (editor_render.rs), the same direct
        // floor project-host's guard sanctions. The az-framework- prefix stays
        // forbidden below.
        "az-project",
        "az-project-host",
        "az-runtime-host",
        "az-session",
        "az-sessiond",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "sample-plugin",
    ];

    fn aether_render_source_files(render_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
        fn collect(directory: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(directory)
                .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
            {
                let path = entry
                    .unwrap_or_else(|error| panic!("read {} entry: {error}", directory.display()))
                    .path();
                if path.is_dir() {
                    collect(&path, files);
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    files.push(path);
                }
            }
        }

        let mut files = Vec::new();
        collect(render_dir, &mut files);
        files.sort();
        files
    }

    fn aether_render_sources(app_dir: &std::path::Path) -> String {
        let render_dir = app_dir.join("aether_editor_view_render");
        aether_render_source_files(&render_dir)
            .into_iter()
            .map(|path| {
                std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    #[test]
    fn production_editor_stays_a_universal_ipc_client() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-editor Cargo.toml");
        let manifest = toml::from_str::<Value>(&manifest).expect("parse az-editor Cargo.toml");
        let workspace_manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
                .expect("read workspace Cargo.toml");
        let workspace_manifest =
            toml::from_str::<Value>(&workspace_manifest).expect("parse workspace Cargo.toml");
        let violations = forbidden_editor_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-editor production dependencies must stay universal and IPC-facing; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn production_editor_rejects_forbidden_workspace_package_aliases() {
        let manifest = toml::from_str::<Value>(
            r"
            [dependencies]
            project-api = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>(
            r#"
            [workspace.dependencies]
            project-api = { package = "sample-plugin", version = "0.1" }
            "#,
        )
        .unwrap();
        let violations = forbidden_editor_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec!["dependencies.project-api(workspace package sample-plugin)"]
        );
    }

    #[test]
    fn production_editor_rejects_legacy_format_and_gem_package_aliases() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            chunk-format = { package = "cry-chunk-assets", version = "0.1" }
            catalog-format = { package = "az-framework-asset-catalog", version = "0.1" }
            object-format = { package = "az-objectstream", version = "0.1" }
            objectstream-format = { package = "az-framework-objectstream", version = "0.1" }
            camera-gem = { package = "az-gem-camera", version = "0.1" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_editor_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "dependencies.camera-gem(package az-gem-camera)",
                "dependencies.catalog-format(package az-framework-asset-catalog)",
                "dependencies.chunk-format(package cry-chunk-assets)",
                "dependencies.object-format(package az-objectstream)",
                "dependencies.objectstream-format(package az-framework-objectstream)",
            ]
        );
    }

    #[test]
    fn production_editor_rejects_legacy_compatibility_crates_by_name() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            az-framework-asset-catalog = "0.1"
            az-framework-objectstream = "0.1"
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_editor_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "dependencies.az-framework-asset-catalog",
                "dependencies.az-framework-objectstream",
            ]
        );
    }

    #[test]
    fn production_editor_rejects_forbidden_path_only_dependencies() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            db = { path = "../crates/az/assetdb" }
            compatible-pak = { path = "../formats/compat/az-pak" }
            terrain = { path = "../gems/legacy-terrain" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_editor_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "dependencies.compatible-pak(path ../formats/compat/az-pak)",
                "dependencies.db(path ../crates/az/assetdb)",
                "dependencies.terrain(path ../gems/legacy-terrain)",
            ]
        );
    }

    #[test]
    fn editor_service_clients_do_not_expose_raw_capnp_escape_hatches() {
        for source_file in [
            "daemon.rs",
            "project_host.rs",
            "runtime_host.rs",
            "session_supervisor.rs",
        ]
        .into_iter()
        .chain(ASSET_PROCESSOR_PARTS.iter().copied())
        {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(source_file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert!(
                !az_architecture_guard::symbols_contain(&source, "pub fn raw_client("),
                "{} must keep Cap'n Proto clients behind typed capability-writing wrappers",
                path.display()
            );
        }
    }

    #[test]
    fn editor_service_raw_capnp_client_constructors_are_test_only() {
        for source_file in [
            "daemon.rs",
            "project_host.rs",
            "asset_processor/client.rs",
            "runtime_host.rs",
            "session_supervisor.rs",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(source_file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert_pub_function_is_cfg_test(&source, "pub fn new(client:", &path);
            assert_pub_function_is_cfg_test(&source, "pub fn with_editor_service(", &path);
            assert_pub_function_is_cfg_test(&source, "pub fn with_capability_templates(", &path);
        }
    }

    #[test]
    fn project_and_runtime_service_clients_require_session_scoped_connect_in_production() {
        for source_file in [
            "project_host.rs",
            "asset_processor/client.rs",
            "runtime_host.rs",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(source_file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert_pub_function_is_cfg_test(&source, "pub async fn connect(", &path);
            assert!(
                az_architecture_guard::symbols_contain(&source, "pub async fn connect_for_session"),
                "{} must expose the production descriptor path as a session-scoped connect",
                path.display()
            );
        }
    }

    #[test]
    fn session_supervisor_discovery_connect_is_attach_discovery_only() {
        let mut direct_connect_violations = Vec::new();
        let mut discovery_connect_violations = Vec::new();

        for source in editor_production_sources() {
            let file_name = source
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("<unknown>");
            if az_architecture_guard::symbols_contain(
                &source.source,
                "SessionSupervisorClient::connect(",
            ) {
                direct_connect_violations.push(file_name.to_string());
            }
            if file_name != "attach.rs"
                && az_architecture_guard::symbols_contain(
                    &source.source,
                    "SessionSupervisorClient::connect_for_discovery(",
                )
            {
                discovery_connect_violations.push(file_name.to_string());
            }
        }

        assert!(
            direct_connect_violations.is_empty(),
            "production editor code must not call unscoped SessionSupervisorClient::connect; daemon attach discovery uses connect_for_discovery and normal paths use connect_for_session: {}",
            direct_connect_violations.join(", ")
        );
        assert!(
            discovery_connect_violations.is_empty(),
            "production editor code may use SessionSupervisorClient::connect_for_discovery only during daemon attach discovery; other paths must use connect_for_session: {}",
            discovery_connect_violations.join(", ")
        );
    }

    #[test]
    fn editor_service_descriptor_preflight_rejects_protocol_skew() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("service_descriptor.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        assert!(
            az_architecture_guard::symbols_contain(&source, "require_protocol_version"),
            "{} must reject stale discovery descriptors before service transport connect \
             (behavioral coverage: DaemonEndpointRecord::require_protocol_version tests)",
            path.display()
        );
    }

    #[test]
    fn editor_service_descriptor_validators_require_brokered_templates() {
        for source_file in [
            "project_host.rs",
            "asset_processor/response_validation.rs",
            "runtime_host.rs",
            "session_supervisor.rs",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(source_file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert!(
                az_architecture_guard::symbols_contain(
                    &source,
                    "validate_descriptor_capability_templates(descriptor,"
                ),
                "{} descriptor validator must reject empty or tokenless descriptor capability templates before opening transport",
                path.display()
            );
        }
    }

    /// `asset_processor` is a directory module (ticket 016); guards that used
    /// to scan the single file scan every part so no seam hides a violation.
    const ASSET_PROCESSOR_PARTS: &[&str] = &[
        "asset_processor/browser_controller.rs",
        "asset_processor/client.rs",
        "asset_processor/event_sink.rs",
        "asset_processor/install.rs",
        "asset_processor/jobs.rs",
        "asset_processor/projection.rs",
        "asset_processor/response_validation.rs",
        "asset_processor/source_validation.rs",
    ];

    /// Every forbidden-needle table in this module pairs each needle with the
    /// reintroduction it must fire on, and the controls iterate the table. The
    /// samples are written out rather than spliced together from the needle: an
    /// interpolated sample matches a needle that can never match real code —
    /// ticket 018's `use az_proto_` prefix is the case that motivated this
    /// (ticket 042).
    ///
    /// State→label and state→tone tables editor-core handed to the shared
    /// `az_editor_ui::status` module (ticket 019). Projections and renderers
    /// read the words off the enum; a private copy here is how "succeeded"
    /// became "done", "built", "Completed", and "stale" on four surfaces.
    const RETIRED_STATUS_TABLES: &[(&str, &str)] = &[
        (
            "asset_entry_status_label",
            "fn asset_entry_status_label(status: AssetEntryStatus) -> &'static str { \"Stale\" }",
        ),
        (
            "asset_job_status_color",
            "fn asset_job_status_color(status: AssetJobStatus, theme: &Theme) -> Hsla { theme.accent }",
        ),
        (
            "asset_job_status_label",
            "fn asset_job_status_label(status: AssetJobStatus) -> &'static str { \"Queued\" }",
        ),
        (
            "job_status_kind",
            "fn job_status_kind(job: &AssetJob) -> &'static str { \"processing\" }",
        ),
        (
            "runtime_state_label",
            "fn runtime_state_label(state: RuntimeState) -> &'static str { \"Stopped\" }",
        ),
        (
            "service_health_state_label",
            "fn service_health_state_label(state: ServiceHealthState) -> &'static str { \"Healthy\" }",
        ),
        (
            "session_state_label",
            "fn session_state_label(state: SessionState) -> &'static str { \"Active\" }",
        ),
    ];

    /// The bounded daemon probe (ticket 043) and the transport-failure
    /// classifier it routes through have exactly one definition each, in
    /// `daemon_bootstrap.rs`. Ticket 047 deleted the two extra classifier copies
    /// — `attach.rs` and the standalone binary — and the binary's own
    /// connect-only probe, which carried no deadline at all: three copies of one
    /// seam is how a bound lands on one path and not the other two.
    ///
    /// Each needle matches a *definition* (`fn name(`), never a use. Calls,
    /// `use` imports, and re-exports name the symbol with no preceding `fn`, so
    /// routing every caller through the one owner stays legal — the control
    /// below pins that. Second column is the deleted copy, written out.
    ///
    /// The binary is inside this scan: `collect_production_rust_sources`
    /// recurses `src/`, which `production_source_scanner_recurses_into_editor_binary`
    /// pins, so a copy reintroduced in `src/bin/az-editor.rs` is caught here.
    const SINGLE_OWNER_DAEMON_PROBE_DEFINITIONS: &[(&str, &str)] = &[
        (
            "fn is_daemon_transport_failure(",
            "fn is_daemon_transport_failure(error: &EditorError) -> bool { matches!(error, EditorError::RpcTransport(_)) }",
        ),
        (
            "fn probe_daemon_endpoint(",
            "fn probe_daemon_endpoint(endpoint: &Endpoint) -> EditorResult<()> { let runtime = tokio::runtime::Builder::new_current_thread().enable_io().enable_time().build()?; let local = tokio::task::LocalSet::new(); let endpoint = endpoint.clone(); local.block_on(&runtime, async move { az_editor::AzDaemonClient::connect(&endpoint).await.map(|_| ()) }) }",
        ),
    ];

    /// The file that must hold the single definition of each needle above.
    const DAEMON_PROBE_OWNER: &str = "daemon_bootstrap.rs";

    /// Project-owned service construction, service hosting, and asset-database
    /// access: all test-only. Production editor code is a universal
    /// descriptor/capability IPC client. Second column is the reintroduction
    /// each needle must still fire on.
    ///
    /// The five `…Rpc::with_` needles this table used to carry could never fire
    /// (ticket 042): [`az_architecture_guard::symbol_skeleton`] keeps
    /// identifier runs whole, so `AzDaemonRpc::with_` cannot match
    /// `AzDaemonRpc::with_shutdown(`. They are respelled as the bare
    /// `AzDaemonRpc::` path, which fires on *every* associated item — the
    /// sibling `::new(` and `::client_from_rc(` entries were folded into it for
    /// the same reason, and the same collapse is why no coverage was lost.
    const FORBIDDEN_PRODUCTION_SNIPPETS: &[(&str, &str)] = &[
        ("AzDaemon::new(", "let daemon = AzDaemon::new();"),
        ("AssetDb::open(", "let db = AssetDb::open(&db_path)?;"),
        (
            "AssetDb::open_in_memory(",
            "let db = AssetDb::open_in_memory()?;",
        ),
        (
            "AssetDb::",
            "let db_path = AssetDb::default_path(&project_root);",
        ),
        (
            "AzDaemonRpc::",
            "let rpc = Rc::new(AzDaemonRpc::with_shutdown(daemon, shutdown, run));",
        ),
        (
            "SessionManager::",
            "let manager = SessionManager::open(&project_root)?;",
        ),
        (
            "az_session::",
            "let manifest = az_session::SessionManifest::load(&path)?;",
        ),
        (
            "ProjectManifest",
            "let manifest: ProjectManifest = read_manifest(&project_root)?;",
        ),
        (
            "load_project_manifest",
            "let manifest = load_project_manifest(&project_root)?;",
        ),
        (
            "load_resolved_project_graph",
            "let graph = load_resolved_project_graph(&manifest)?;",
        ),
        (
            "ProjectHost::new(",
            "let host = ProjectHost::new(source_root, capability_grants);",
        ),
        (
            "ProjectHost::with_source_root",
            "let host = ProjectHost::with_source_root(&project_root);",
        ),
        (
            "ProjectHost::open_source_root",
            "let host = ProjectHost::open_source_root(&project_root)?;",
        ),
        (
            "ProjectHost::open_project_source_root",
            "let host = ProjectHost::open_project_source_root(&project_root, &project_id)?;",
        ),
        (
            "ProjectHostRpc::",
            "let rpc = ProjectHostRpc::with_side_channel_root_and_session(host, root, None, None, grants, composition, authoring);",
        ),
        (
            "AssetProcessor::new(",
            "let processor = AssetProcessor::new(workspace);",
        ),
        (
            "AssetProcessorRpc::",
            "let rpc = AssetProcessorRpc::with_db(db, capability_grants);",
        ),
        (
            "az_asset_processor::asset_processor_client(",
            "let client = az_asset_processor::asset_processor_client(&rpc);",
        ),
        (
            "RuntimeHost::new(",
            "let host = RuntimeHost::new(registries);",
        ),
        (
            "RuntimeHostRpc::",
            "let rpc = RuntimeHostRpc::with_service_run(host, service_run);",
        ),
        (
            "SessionSupervisorRpc::",
            "let rpc = SessionSupervisorRpc::with_manager_and_command_sender(manager, sender);",
        ),
        (
            "Endpoint::in_process(",
            "let endpoint = Endpoint::in_process(\"editor\");",
        ),
        (
            "EndpointKind::InProcess",
            "let kind = EndpointKind::InProcess;",
        ),
        (
            "start_az_daemon_rpc_server",
            "let server = start_az_daemon_rpc_server(daemon, endpoint)?;",
        ),
        (
            "start_project_host_rpc_server",
            "let server = start_project_host_rpc_server(host, endpoint)?;",
        ),
        (
            "start_asset_processor_rpc_server",
            "let server = start_asset_processor_rpc_server(processor, endpoint)?;",
        ),
        (
            "start_runtime_host_rpc_server",
            "let server = start_runtime_host_rpc_server(host, endpoint)?;",
        ),
        (
            "start_session_supervisor_rpc_server",
            "let server = start_session_supervisor_rpc_server(manager, endpoint)?;",
        ),
    ];

    /// Local shortcuts around daemon/session-supervisor discovery that editor
    /// attach must never take.
    const ATTACH_DISCOVERY_BYPASSES: &[(&str, &str)] = &[
        (
            "SessionManager::",
            "let manager = SessionManager::open(&project_root)?;",
        ),
        (
            "az_session::",
            "let manifest = az_session::SessionManifest::load(&path)?;",
        ),
        (
            "ProjectManifest",
            "let manifest: ProjectManifest = read_manifest(&project_root)?;",
        ),
        (
            "load_project_manifest",
            "let manifest = load_project_manifest(&project_root)?;",
        ),
        ("AssetDb::", "let db = AssetDb::open(&db_path)?;"),
        (
            "ProjectHostRpc::",
            "let rpc = ProjectHostRpc::test_new(host, capability_grants);",
        ),
        (
            "AssetProcessorRpc::",
            "let rpc = AssetProcessorRpc::with_db(db, capability_grants);",
        ),
        (
            "RuntimeHostRpc::",
            "let rpc = RuntimeHostRpc::test_new(host);",
        ),
        (
            "Endpoint::in_process",
            "let endpoint = Endpoint::in_process(\"editor-attach\");",
        ),
        (
            "std::process::Command",
            "std::process::Command::new(\"az-project-host\").spawn()?;",
        ),
    ];

    /// Process spawning the editor console must not reach for; commands run
    /// through `SessionSupervisor.execCommand`.
    const CONSOLE_PROCESS_BYPASSES: &[(&str, &str)] = &[
        ("std::process", "use std::process::Command;"),
        (
            "Command::new",
            "let child = Command::new(\"az\").arg(\"build\").spawn()?;",
        ),
        (
            "run_external_command",
            "let output = run_external_command(&command_line)?;",
        ),
    ];

    /// The local attach path the standalone binary must not fall back to.
    const STANDALONE_LOCAL_ATTACH_FALLBACKS: &[(&str, &str)] = &[
        (
            "EditorApp::open_session(project_root",
            "EditorApp::open_session(project_root, cx)?;",
        ),
        (
            "should_use_daemon_endpoint",
            "if should_use_daemon_endpoint(&args) { attach_locally(project_root, cx)?; }",
        ),
    ];

    /// CPU pixel-file side-channel reading the editor runtime-host must not own;
    /// brokered GPU surface handles are validated through the shared readers.
    const RUNTIME_HOST_SIDE_CHANNEL_FILE_READERS: &[(&str, &str)] = &[
        (
            "fn read_mmap_file_side_channel",
            "fn read_mmap_file_side_channel(handle: &SideChannelHandle) -> EditorResult<Vec<u8>> {",
        ),
        (
            "read_verified_staging_file(",
            "let pixels = read_verified_staging_file(&handle.locator, expected_len)?;",
        ),
        (
            "read_mmap_file_prefix(",
            "let header = read_mmap_file_prefix(&path, 64)?;",
        ),
        (
            "PathBuf::from(&handle.locator)",
            "let path = PathBuf::from(&handle.locator);",
        ),
        ("std::fs::File", "let file = std::fs::File::open(&path)?;"),
        ("std::io::Read", "use std::io::Read;"),
    ];

    /// Hover style keys `AetherStyle` must not carry; GPUI's `.hover` owns hover
    /// state. `AetherStyle` is a string map, so these are contract text.
    const AETHER_IMPLICIT_HOVER_STYLE_KEYS: &[(&str, &str)] = &[
        (
            "hoverBackground",
            "let background = row.style.get(\"hoverBackground\").unwrap_or(base);",
        ),
        (
            "hoverColor",
            "let color = row.style.get(\"hoverColor\").unwrap_or(base);",
        ),
        (
            "hoverBorderColor",
            "let border = row.style.get(\"hoverBorderColor\").unwrap_or(base);",
        ),
    ];

    /// Raw model children the Aether renderer must not reach into; it consumes
    /// a state projection or a command instead.
    const AETHER_RAW_STATE_CHILDREN: &[(&str, &str)] = &[
        (
            "self.state.workspace",
            "let workspace = self.state.workspace.clone();",
        ),
        (
            "self.state.overlay",
            "let overlay = self.state.overlay.active_modal();",
        ),
        (
            "self.state.assets",
            "let assets = self.state.assets.entries();",
        ),
        (
            "self.state.authored_content",
            "let authored = self.state.authored_content.expansion();",
        ),
        (
            "self.state.game_data",
            "let catalog = self.state.game_data.tables();",
        ),
        ("self.state.gems", "let gems = self.state.gems.installed();"),
        (
            "self.state.diagnostics",
            "let console = self.state.diagnostics.console();",
        ),
    ];

    /// Selection, mutation, and RPC authority the Aether authored-content
    /// presentation state must not own.
    const AETHER_AUTHORED_CONTENT_AUTHORITY: &[(&str, &str)] = &[
        (
            "EditorReflectedSelectionState",
            "let selection = cx.try_global::<EditorReflectedSelectionState>();",
        ),
        (
            "ReflectedEntityInspection",
            "pub inspection: Option<ReflectedEntityInspection>,",
        ),
        (
            "ReflectedEditBinding",
            "pub bindings: Vec<ReflectedEditBinding>,",
        ),
        ("PrefabEditCommand", "pub pending: Vec<PrefabEditCommand>,"),
        (
            "SourceAuthoring",
            "let authoring = SourceAuthoring::new(client);",
        ),
        ("ProjectHostClient", "pub client: ProjectHostClient,"),
        (
            "AssetProcessorClient",
            "pub processor: AssetProcessorClient,",
        ),
    ];

    /// Controller truth and background/client state the Aether diagnostics
    /// presentation state must not mirror.
    const AETHER_DIAGNOSTICS_CONTROLLER_MIRRORS: &[(&str, &str)] = &[
        (
            "EditorRuntimeStatus",
            "let status = cx.try_global::<EditorRuntimeStatus>();",
        ),
        (
            "EditorMannequinPreview",
            "pub preview: Option<EditorMannequinPreview>,",
        ),
        (
            "AssetProcessorClient",
            "pub processor: AssetProcessorClient,",
        ),
        ("ProjectHostClient", "pub client: ProjectHostClient,"),
        (
            "std::thread",
            "std::thread::spawn(move || tail_service_log(path));",
        ),
        (
            "tokio",
            "tokio::spawn(async move { tail_service_log(path).await });",
        ),
    ];

    /// Publishers retired by the typed `ModeProjectionSpec` lifecycle.
    const RETIRED_MODE_PROJECTION_PUBLISHERS: &[(&str, &str)] = &[
        (
            "republish_materials_projection",
            "pub(crate) fn republish_materials_projection(cx: &mut App) { publish(cx); }",
        ),
        (
            "republish_scripting_projection",
            "pub(crate) fn republish_scripting_projection(cx: &mut App) { publish(cx); }",
        ),
        (
            "republish_game_data_projection",
            "pub(crate) fn republish_game_data_projection(cx: &mut App) { publish(cx); }",
        ),
    ];

    /// Reverse republish edges upstream files must not call. These were spelled
    /// `materials_ui::republish` and could never fire (ticket 042): identifier
    /// runs stay whole, so a module path plus a *prefix* of the function name
    /// never matches the call it names. They are respelled as the full retired
    /// publisher path.
    const RETIRED_MODE_PROJECTION_REPUBLISH_EDGES: &[(&str, &str)] = &[
        (
            "materials_ui::republish_materials_projection",
            "crate::materials_ui::republish_materials_projection(cx);",
        ),
        (
            "scripting_ui::republish_scripting_projection",
            "crate::scripting_ui::republish_scripting_projection(cx);",
        ),
        (
            "game_data_catalog::republish_game_data_projection",
            "crate::game_data_catalog::republish_game_data_projection(cx);",
        ),
    ];

    /// Generated `renderVals` design-seed accessors the Aether editor must not
    /// use; the renderer reads typed state projections. Needle and sample are
    /// both assembled with `concat!` so this table never spells the retired
    /// names as literal source text.
    const AETHER_GENERATED_DESIGN_SEED: &[(&str, &str)] = &[
        (
            concat!("RENDER", "_VALS_JSON"),
            concat!(
                "const RENDER",
                "_VALS_JSON: &str = include_str!(\"seed.json\");"
            ),
        ),
        (
            concat!("render", "_value"),
            concat!(
                "fn render",
                "_value(vals: &Map<String, Value>, key: &str) -> Option<String> { vals.get(key) }"
            ),
        ),
        (
            concat!("render", "_string"),
            concat!(
                "let title = render",
                "_string(vals, \"title\").unwrap_or_default();"
            ),
        ),
        (
            concat!("render", "_bool"),
            concat!(
                "if render",
                "_bool(vals, \"enabled\") { row = row.opacity(1.0); }"
            ),
        ),
        (
            concat!("render", "_vals"),
            concat!("let vals = render", "_vals(\"titlebar\");"),
        ),
    ];

    /// The generated seed file name, checked as contract text.
    const AETHER_GENERATED_DESIGN_SEED_FILE: &str =
        concat!("aether_editor", "_render", "_vals", ".json");

    const AETHER_GENERATED_DESIGN_SEED_FILE_REINTRODUCTION: &str = concat!(
        "const SEED: &str = include_str!(\"aether_editor",
        "_render",
        "_vals",
        ".json\");"
    );

    /// View state and service clients a private project-manager render region
    /// must not own; regions render from `ProjectManagerView` projections and
    /// actions.
    const PROJECT_MANAGER_REGION_STATE_AND_CLIENTS: &[(&str, &str)] = &[
        (
            "pub struct ProjectManagerView",
            "pub struct ProjectManagerView { pub recent: Vec<RecentProject> }",
        ),
        (
            "ProjectHostClient",
            "let client = ProjectHostClient::connect_for_session(&descriptor, &session_id).await?;",
        ),
        (
            "AssetProcessorClient",
            "let client = AssetProcessorClient::connect_for_session(&descriptor, &session_id).await?;",
        ),
        (
            "std::thread",
            "std::thread::spawn(move || refresh_recent_projects(view));",
        ),
        (
            "tokio",
            "tokio::spawn(async move { refresh_recent_projects(view).await });",
        ),
    ];

    /// Renderers that moved out of `ProjectManagerView` into private regions.
    const REMOVED_PROJECT_MANAGER_RENDERERS: &[(&str, &str)] = &[
        (
            "fn render_manager_nav_rail(",
            "fn render_manager_nav_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {",
        ),
        (
            "fn render_manager_recent_view(",
            "fn render_manager_recent_view(&self, cx: &mut Context<Self>) -> impl IntoElement {",
        ),
        (
            "fn render_titlebar(",
            "fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {",
        ),
    ];

    /// The descriptor/capability IPC-client shapes production editor code does
    /// use. No needle above may match one of these.
    const EDITOR_IPC_CLIENT_SHAPES: &[&str] = &[
        "let client = ProjectHostClient::connect_for_session(&descriptor, &session_id).await?;",
        "let client = AssetProcessorClient::connect_for_session(&descriptor, &session_id).await?;",
        "let supervisor = SessionSupervisorClient::connect_for_discovery(&endpoint).await?;",
        "let descriptor = resolve_project_host_descriptor(&supervisor, &session_slug).await?;",
    ];

    #[test]
    fn status_labels_and_tones_come_from_the_shared_ui_module() {
        let sources = az_architecture_guard::collect_production_rust_sources(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src"
        ))
        .expect("collect az-editor production sources");
        let mut violations = Vec::new();
        for source in &sources {
            for &(retired, _) in RETIRED_STATUS_TABLES {
                if az_architecture_guard::symbols_contain(&source.source, retired) {
                    violations.push(format!("{} names `{retired}`", source.path.display()));
                }
            }
        }

        assert!(
            !sources.is_empty(),
            "status guard scanned no editor sources"
        );
        assert!(
            violations.is_empty(),
            "editor-core renders the shared status vocabulary; label and tone tables live in az-editor-ui `status.rs`:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the guard above: every needle must still match a
    /// table that reintroduces it, and none may match the shared method calls
    /// that replaced them.
    #[test]
    fn retired_status_table_needles_fire_and_spare_the_shared_methods() {
        for &(retired, reintroduction) in RETIRED_STATUS_TABLES {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, retired),
                "needle `{retired}` cannot match the table it forbids: {reintroduction}"
            );
            for call in [
                "job.status.label().to_owned()",
                "entry.status.tone().color(theme)",
                "activity.state.label()",
                "hsla_css(phase.tone().color(theme))",
            ] {
                assert!(
                    !az_architecture_guard::symbols_contain(call, retired),
                    "needle `{retired}` rejects the shared status method call `{call}`"
                );
            }
        }
    }

    #[test]
    fn the_bounded_daemon_probe_and_its_classifier_have_one_owner() {
        let sources = editor_production_sources();
        assert!(
            !sources.is_empty(),
            "daemon probe guard scanned no editor sources"
        );

        for &(needle, _) in SINGLE_OWNER_DAEMON_PROBE_DEFINITIONS {
            let mut definitions = Vec::new();
            for source in &sources {
                for _ in 0..az_architecture_guard::symbols_count(&source.source, needle) {
                    definitions.push(source.path.display().to_string());
                }
            }

            assert_eq!(
                definitions.len(),
                1,
                "`{needle}` must have exactly one owner; found {} definition(s): {}",
                definitions.len(),
                definitions.join(", ")
            );
            assert!(
                definitions[0].ends_with(DAEMON_PROBE_OWNER),
                "`{needle}` belongs to {DAEMON_PROBE_OWNER}; callers import it instead of redefining it, but it was defined in {}",
                definitions[0]
            );
        }
    }

    /// Positive control for the guard above: every needle fires on the copy it
    /// forbids, and none fires on the imports and call sites that route the
    /// former copy-holders through the single owner.
    #[test]
    fn retired_daemon_probe_copy_needles_fire_and_spare_the_shared_calls() {
        for &(needle, reintroduction) in SINGLE_OWNER_DAEMON_PROBE_DEFINITIONS {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the copy it forbids: {reintroduction}"
            );
            for shared in [
                "use az_editor::daemon_bootstrap::{is_daemon_transport_failure, probe_daemon_endpoint};",
                "use crate::daemon_bootstrap::is_daemon_transport_failure;",
                "pub use daemon_bootstrap::probe_daemon_endpoint;",
                "match probe_daemon_endpoint(&endpoint) { Ok(()) => {} Err(error) => return Err(error) }",
                "Err(error) if is_daemon_transport_failure(&error) => {}",
                "if probe_daemon_endpoint(&record.endpoint).is_ok() { return Ok(record); }",
            ] {
                assert!(
                    !az_architecture_guard::symbols_contain(shared, needle),
                    "needle `{needle}` rejects the single-owner call site `{shared}`"
                );
            }
        }
    }

    /// `editor_render` keeps the Bevy app orchestration while its independent
    /// `DComp`, picking, gizmo, preview, and camera capabilities stay in their
    /// named subsystem modules (ticket 024).
    const EDITOR_RENDER_PARTS: &[(&str, &str)] = &[
        ("editor_render/dcomp_present.rs", "struct DcompPresentFence"),
        ("editor_render/pick.rs", "struct EditorPickProxy"),
        ("editor_render/gizmo.rs", "enum GizmoMode"),
        (
            "editor_render/mannequin_preview.rs",
            "struct MannequinPreviewEntity",
        ),
        ("editor_render/camera.rs", "struct OrbitCameraController"),
    ];

    #[test]
    fn editor_render_keeps_subsystems_out_of_the_orchestrator() {
        let source_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let orchestrator_path = source_root.join("editor_render.rs");
        let orchestrator = std::fs::read_to_string(&orchestrator_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", orchestrator_path.display()));

        for (part, required_symbol) in EDITOR_RENDER_PARTS {
            let path = source_root.join(part);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert!(
                source.lines().count() <= 1_000,
                "{} must remain a reviewable editor-render subsystem, not another monolith",
                path.display()
            );
            assert!(
                az_architecture_guard::symbols_contain(&source, required_symbol),
                "{} must own `{required_symbol}`",
                path.display()
            );
            assert!(
                !az_architecture_guard::symbols_contain(&orchestrator, required_symbol),
                "editor_render.rs must orchestrate instead of reabsorbing `{required_symbol}`"
            );
        }
    }

    /// Each private render region file and the renderer it must own.
    const REGIONS: &[(&str, &str)] = &[
        (
            "project_manager_view/titlebar.rs",
            "fn render_titlebar_region(",
        ),
        (
            "project_manager_view/nav_rail.rs",
            "fn render_nav_rail_region(",
        ),
        ("project_manager_view/recent.rs", "fn render_recent_region("),
        (
            "project_manager_view/new_project.rs",
            "fn render_new_project_region(",
        ),
        ("project_manager_view/footer.rs", "fn render_footer_region("),
        (
            "project_manager_view/summary.rs",
            "fn render_summary_region(",
        ),
        (
            "project_manager_view/placeholder.rs",
            "fn render_placeholder_region(",
        ),
    ];

    #[test]
    fn project_manager_view_composes_private_render_regions() {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app");
        let composer_path = app_dir.join("project_manager_view.rs");
        let composer = std::fs::read_to_string(&composer_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", composer_path.display()));
        let model_path = app_dir.join("project_manager.rs");
        let model = std::fs::read_to_string(&model_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", model_path.display()));

        assert!(
            composer.lines().count() <= 120,
            "{} must remain a short region composer",
            composer_path.display()
        );
        for required in [
            "fn render_aether_manager(",
            "titlebar::render_titlebar_region(",
            "nav_rail::render_nav_rail_region(",
            "recent::render_recent_region(",
            "new_project::render_new_project_region(",
            "placeholder::render_placeholder_region(",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&composer, required),
                "{} must compose `{required}`",
                composer_path.display()
            );
        }
        assert_eq!(
            az_architecture_guard::symbols_count(&model, "pub struct ProjectManagerView"),
            1,
            "{} must remain the sole ProjectManagerView state owner",
            model_path.display()
        );
        for &(removed_renderer, _) in REMOVED_PROJECT_MANAGER_RENDERERS {
            assert!(
                !az_architecture_guard::symbols_contain(&model, removed_renderer),
                "{} must retain state, projections, and actions only; `{removed_renderer}` belongs to a private render region",
                model_path.display()
            );
        }

        assert_each_region_owns_its_renderer(&app_dir);
        assert_each_region_owns_its_implementation(&app_dir);
        assert_new_project_region_composes_footer_and_summary(&app_dir);
    }

    /// Every private render region stays reviewable and owns the renderer
    /// the composer names, holding no view state or service client itself.
    fn assert_each_region_owns_its_renderer(app_dir: &std::path::Path) {
        for (relative_path, required_symbol) in REGIONS {
            let path = app_dir.join(relative_path);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert!(
                source.lines().count() <= 1_000,
                "{} must remain a reviewable project-manager region",
                path.display()
            );
            assert!(
                az_architecture_guard::symbols_contain(&source, required_symbol),
                "{} must own `{required_symbol}`",
                path.display()
            );
            for &(forbidden, _) in PROJECT_MANAGER_REGION_STATE_AND_CLIENTS {
                assert!(
                    !az_architecture_guard::symbols_contain(&source, forbidden),
                    "{} must render from ProjectManagerView projections and actions, not `{forbidden}`",
                    path.display()
                );
            }
        }
    }

    /// Each region renders itself instead of forwarding the work back to
    /// `ProjectManagerView`.
    fn assert_each_region_owns_its_implementation(app_dir: &std::path::Path) {
        for (relative_path, implementation) in [
            ("project_manager_view/titlebar.rs", "TitleBar::new("),
            ("project_manager_view/nav_rail.rs", "view.nav_items()"),
            ("project_manager_view/recent.rs", "view.recent_projects()"),
        ] {
            let path = app_dir.join(relative_path);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert!(
                az_architecture_guard::symbols_contain(&source, implementation),
                "{} must own its render implementation rather than forward `{implementation}` to ProjectManagerView",
                path.display()
            );
        }
    }

    /// The New Project region is itself a composer of two smaller regions.
    fn assert_new_project_region_composes_footer_and_summary(app_dir: &std::path::Path) {
        let new_project_path = app_dir.join("project_manager_view/new_project.rs");
        let new_project = std::fs::read_to_string(&new_project_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", new_project_path.display()));
        for required in [
            "footer::render_footer_region(",
            "summary::render_summary_region(",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&new_project, required),
                "{} must compose `{required}`",
                new_project_path.display()
            );
        }
    }

    /// Positive control for the guard above: every needle fires on the state,
    /// client, or reabsorbed renderer it forbids, and none of them matches the
    /// private render regions the composer is built from.
    #[test]
    fn project_manager_region_needles_fire_and_spare_the_region_shapes() {
        const REGION_SHAPES: &[&str] = &[
            "fn render_titlebar_region(view: &ProjectManagerView, cx: &mut App) -> impl IntoElement",
            "fn render_nav_rail_region(view: &ProjectManagerView, cx: &mut App) -> impl IntoElement",
            "recent::render_recent_region(view, cx)",
        ];

        for &(needle, reintroduction) in PROJECT_MANAGER_REGION_STATE_AND_CLIENTS
            .iter()
            .chain(REMOVED_PROJECT_MANAGER_RENDERERS)
        {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the shape it forbids: {reintroduction}"
            );
            for shape in REGION_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the private render region `{shape}`"
                );
            }
        }
    }

    #[test]
    fn production_editor_does_not_construct_project_owned_services_or_asset_databases() {
        let mut violations = Vec::new();

        for source in editor_production_sources() {
            for &(forbidden, _) in FORBIDDEN_PRODUCTION_SNIPPETS {
                if az_architecture_guard::symbols_contain(&source.source, forbidden) {
                    violations.push(format!(
                        "{} contains `{forbidden}`",
                        source
                            .path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("<unknown>")
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "az-editor production code must stay a universal descriptor/capability IPC client; direct project-owned service construction, service hosting, and asset DB access are test-only:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the guard above: every needle fires on the
    /// construction it forbids, and none of them matches the descriptor-backed
    /// IPC client shapes production editor code is built from.
    #[test]
    fn production_service_construction_needles_fire_and_spare_the_ipc_client_shapes() {
        for &(needle, reintroduction) in FORBIDDEN_PRODUCTION_SNIPPETS {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the construction it forbids: {reintroduction}"
            );
            for shape in EDITOR_IPC_CLIENT_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the IPC client shape `{shape}`"
                );
            }
        }
    }

    #[test]
    fn direct_project_host_source_save_wrappers_are_test_only() {
        for (source_file, functions) in [
            (
                "project_host.rs",
                &[
                    "pub async fn save_document(",
                    "pub async fn save_document_record(",
                ][..],
            ),
            (
                "authored_edit.rs",
                &[
                    "pub async fn save_document(",
                    "pub async fn save_inspected_document(",
                ][..],
            ),
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(source_file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            for function in functions {
                assert_pub_function_is_cfg_test(&source, function, &path);
            }
        }
    }

    #[test]
    fn project_host_edit_session_direct_client_harness_is_test_only() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("authored_edit.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        for function in [
            "pub fn new(client: ProjectHostClient",
            "pub fn client(&self)",
            "pub fn with_session_supervisor(",
        ] {
            assert_pub_function_is_cfg_test(&source, function, &path);
        }
    }

    #[test]
    fn asset_browser_direct_client_harness_is_test_only() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("asset_processor/browser_controller.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        for function in [
            "pub fn new(client: AssetProcessorClient",
            "pub fn with_session_supervisor(",
        ] {
            assert_pub_function_is_cfg_test(&source, function, &path);
        }
    }

    #[test]
    fn runtime_controller_direct_client_harness_is_test_only() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("runtime_host.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        for function in [
            "pub fn new(",
            "pub const fn with_session_id(",
            "pub fn with_asset_processor(",
            "pub fn with_session_supervisor(",
        ] {
            assert_pub_function_is_cfg_test(&source, function, &path);
        }
    }

    #[test]
    fn low_level_service_clients_do_not_expose_public_raw_scope_or_grant_mutators() {
        for source_file in [
            "daemon.rs",
            "project_host.rs",
            "asset_processor/client.rs",
            "runtime_host.rs",
            "session_supervisor.rs",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join(source_file);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

            assert!(
                !az_architecture_guard::symbols_contain(
                    &source,
                    "pub const fn with_session_scope("
                ),
                "{} must not expose raw session-scoping mutation publicly; production callers use descriptor-backed connect_for_session",
                path.display()
            );
            for function in [
                "pub fn with_editor_service(",
                "pub fn with_capability_templates(",
            ] {
                assert_pub_function_is_cfg_test(&source, function, &path);
            }
        }
    }

    #[test]
    fn editor_console_commands_stay_on_session_supervisor_exec_boundary() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("console.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        assert!(
            az_architecture_guard::symbols_contain(&source, ".exec_command"),
            "editor console commands must execute through SessionSupervisor.execCommand"
        );
        for &(forbidden, _) in CONSOLE_PROCESS_BYPASSES {
            assert!(
                !az_architecture_guard::symbols_contain(&source, forbidden),
                "editor console must not bypass session-supervisor with `{forbidden}`"
            );
        }
    }

    #[test]
    fn standalone_editor_startup_has_no_project_attach_local_fallback() {
        // The binary is `fn main` alone; its startup sequence is the library
        // module this guard scans.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("startup.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let production_source = production_source_without_cfg_test_modules(&source);

        for &(forbidden, _) in STANDALONE_LOCAL_ATTACH_FALLBACKS {
            assert!(
                !az_architecture_guard::symbols_contain(&production_source, forbidden),
                "standalone az-editor project attach must not fall back to a local attach path: `{forbidden}`"
            );
        }
        assert!(
            az_architecture_guard::symbols_contain(
                &production_source,
                "EditorApp::open_project_session_via_daemon"
            ),
            "standalone az-editor project attach must go through the daemon-owned project workflow lifecycle"
        );
    }

    #[test]
    fn project_bound_editor_launch_uses_background_open_shell() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let production_source = production_source_without_cfg_test_modules(&source);

        assert!(
            az_architecture_guard::symbols_contain(
                &production_source,
                "open_project_session_via_daemon"
            ) && az_architecture_guard::symbols_contain(
                &production_source,
                "PendingProjectWorkflowOpen"
            ) && az_architecture_guard::symbols_contain(&production_source, "OpenEditorSession"),
            "project-bound editor launch must create the GPUI shell and open the project/session in the background workflow"
        );
        assert!(
            !az_architecture_guard::symbols_contain(
                &production_source,
                "let outcome = open_project_workflow_editor_session_with_daemon_endpoint"
            ),
            "project-bound editor launch must not synchronously attach before the first editor window can be created"
        );
    }

    #[test]
    fn aether_editor_model_has_no_generated_design_seed() {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app");
        let model =
            std::fs::read_to_string(app_dir.join("aether_editor_model.rs")).expect("read model");
        let view =
            std::fs::read_to_string(app_dir.join("aether_editor_view.rs")).expect("read view");
        let render = aether_render_sources(&app_dir);
        let source = model + &view + &render;

        for &(forbidden, _) in AETHER_GENERATED_DESIGN_SEED {
            assert!(
                !az_architecture_guard::symbols_contain(&source, forbidden),
                "Aether editor must not use generated renderVals data: `{forbidden}`"
            );
        }
        assert!(
            !az_architecture_guard::source_literal_contains(
                &source,
                AETHER_GENERATED_DESIGN_SEED_FILE
            ),
            "Aether editor must not embed the generated renderVals seed file: `{AETHER_GENERATED_DESIGN_SEED_FILE}`"
        );
    }

    /// Positive control for the guard above: every needle fires on the
    /// generated-seed accessor it forbids, and none of them matches the typed
    /// projections the Aether renderer reads instead.
    ///
    /// The seed *file name* is checked as contract text: it only ever appears
    /// inside an `include_str!` literal, whose contents
    /// [`az_architecture_guard::symbol_skeleton`] strips, so as a symbol needle
    /// it could never fire (ticket 042).
    #[test]
    fn aether_design_seed_needles_fire_and_spare_the_typed_projections() {
        const TYPED_PROJECTION_SHAPES: &[&str] = &[
            "let row = SettingsRowProjection::from_state(&self.state, cx);",
            "let counts = AssetPipelineCounts::from_jobs(&projection.jobs);",
        ];

        for &(needle, reintroduction) in AETHER_GENERATED_DESIGN_SEED {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the generated-seed use it forbids: {reintroduction}"
            );
            for shape in TYPED_PROJECTION_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the typed projection `{shape}`"
                );
            }
        }
        assert!(
            az_architecture_guard::source_literal_contains(
                AETHER_GENERATED_DESIGN_SEED_FILE_REINTRODUCTION,
                AETHER_GENERATED_DESIGN_SEED_FILE
            ),
            "the seed file needle cannot match the embed it forbids"
        );
        assert!(
            !az_architecture_guard::source_literal_contains(
                "let row = SettingsRowProjection::from_state(&self.state, cx);",
                AETHER_GENERATED_DESIGN_SEED_FILE
            ),
            "the seed file needle rejects the typed projection that replaced it"
        );
    }

    #[test]
    fn aether_top_menu_isolated_region_uses_kit_dropdowns() {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app");
        let view =
            std::fs::read_to_string(app_dir.join("aether_editor_view.rs")).expect("read view");

        assert!(
            az_architecture_guard::symbols_contain(&view, "struct AetherTitlebarRegion")
                && az_architecture_guard::symbols_contain(&view, "render_top_menu_dropdown"),
            "top-menu titles must live in the isolated titlebar region and use kit dropdowns"
        );
        assert!(
            az_architecture_guard::symbols_contain(&view, "PopupMenuItem")
                && az_architecture_guard::symbols_contain(&view, "activate_menu_item"),
            "isolated top-menu rows must retain model action routing"
        );
        let render = aether_render_sources(&app_dir);
        assert!(
            az_architecture_guard::symbols_contain(&render, "AetherMenuAction")
                && az_architecture_guard::symbols_contain(&render, "dispatch_menu_action")
                && az_architecture_guard::symbols_contain(&render, "window.dispatch_action"),
            "enabled Aether menu rows must carry typed commands into GPUI dispatch"
        );
        assert!(
            !az_architecture_guard::symbols_contain(&render, "activate_item_with_window"),
            "workspace activation must have one window-aware model-to-layout call stack"
        );
    }

    #[test]
    fn aether_toolbar_cluster_routes_build_and_preferences() {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app");
        let render = aether_render_sources(&app_dir);
        let view =
            std::fs::read_to_string(app_dir.join("aether_editor_view.rs")).expect("read view");

        assert!(
            az_architecture_guard::source_literal_contains(&render, "execute-build")
                && az_architecture_guard::symbols_contain(
                    &render,
                    "az_editor_ui::actions::ExecuteProjectBuild"
                ),
            "Build action rows must dispatch ExecuteProjectBuild"
        );
        assert!(
            az_architecture_guard::symbols_contain(&view, "open_settings"),
            "toolbar settings gear must open Preferences"
        );
    }

    #[test]
    fn editor_shell_separates_project_launcher_from_loaded_project_workspace() {
        let sources = EditorShellSources::read();
        assert_shell_switches_between_launcher_and_workspace(&sources);
        assert_loaded_dock_is_profile_driven(&sources);
        assert_scene_profile_dock_membership();
        assert_adopted_workspace_regions(&sources);
        assert_menu_and_keymap_expose_the_graph_workbench(&sources);
    }

    /// The sources the shell/workspace separation guard reads.
    struct EditorShellSources {
        app: String,
        dock: String,
        layout: String,
        keymaps: String,
        menu: String,
    }

    impl EditorShellSources {
        fn read() -> Self {
            // The shell (`app.rs`) and adopted Aether editor workspace modules are
            // separate; concatenate them so these source-level guards hold
            // regardless of which file an item lives in.
            let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
            let app = std::fs::read_to_string(app_dir.join("app.rs")).expect("read editor app.rs")
                + &std::fs::read_to_string(app_dir.join("app").join("aether_editor_view.rs"))
                    .expect("read editor app/aether_editor_view.rs")
                + &std::fs::read_to_string(app_dir.join("app").join("aether_editor_model.rs"))
                    .expect("read editor app/aether_editor_model.rs")
                + &aether_render_sources(&app_dir.join("app"));
            let dock = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src")
                    .join("workspace")
                    .join("dock.rs"),
            )
            .expect("read editor workspace dock.rs");
            let layout = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src")
                    .join("workspace")
                    .join("layout.rs"),
            )
            .expect("read editor workspace layout.rs");
            let keymaps = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src")
                    .join("keymaps.rs"),
            )
            .expect("read editor keymaps.rs");
            let menu = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("..")
                    .join("ui")
                    .join("src")
                    .join("menu")
                    .join("mod.rs"),
            )
            .expect("read editor menu mod.rs");

            Self {
                app,
                dock,
                layout,
                keymaps,
                menu,
            }
        }
    }

    /// The shell shows the launcher until a project connects, and stays in
    /// the loaded workspace when the open workflow fails.
    fn assert_shell_switches_between_launcher_and_workspace(sources: &EditorShellSources) {
        let app = &sources.app;
        assert!(
            az_architecture_guard::symbols_contain(app, "pub struct AppShell")
                && az_architecture_guard::symbols_contain(app, "ProjectManagerView")
                && az_architecture_guard::symbols_contain(app, "workspace: Option<Entity<EditorView>>")
                && az_architecture_guard::symbols_contain(app, "project_workflow::Status")
                && az_architecture_guard::symbols_contain(app, "editor_project_host_connecting(cx)")
                // The shell transitions into the loaded workspace as soon as a
                // project starts connecting (editor-shell-immediately), not only
                // once a session is fully attached.
                && az_architecture_guard::symbols_contain(app, "if attached || connecting"),
            "app shell must use ProjectManagerView when unattached and transition to the adopted EditorView once a project session connects/attaches"
        );
        assert!(
            az_architecture_guard::symbols_contain(app, "editor_project_host_failed_reason(cx)")
                && az_architecture_guard::symbols_contain(
                    app,
                    "if attached || connecting || failed"
                ),
            "app shell must stay in the loaded workspace when the open workflow fails so gated \
             panels can render the failure reason and a way back, instead of silently bouncing \
             to the launcher"
        );
    }

    /// The loaded dock and its layout profiles are data-driven, and project
    /// workflow is a panel in neither.
    fn assert_loaded_dock_is_profile_driven(sources: &EditorShellSources) {
        let dock = &sources.dock;
        let layout = &sources.layout;
        assert!(
            az_architecture_guard::symbols_contain(dock, "layout_profile_for_mode")
                && az_architecture_guard::symbols_contain(dock, "layout.center")
                && az_architecture_guard::symbols_contain(dock, "dock_tabs_for_workbenches")
                && !az_architecture_guard::symbols_contain(
                    dock,
                    "az_editor_ui::panels::ProjectWorkflowPanel::NAME"
                )
                && !az_architecture_guard::symbols_contain(
                    dock,
                    "vec![Arc::from(graph), Arc::from(viewport)]"
                )
                && !az_architecture_guard::symbols_contain(dock, "Arc::from(project_workflow)"),
            "loaded-project dock must be profile-driven, default to the viewport workbench, and keep project workflow out of the loaded workspace"
        );
        assert!(
            az_architecture_guard::symbols_contain(layout, "WorkbenchKind::LevelViewport")
                && az_architecture_guard::symbols_contain(
                    layout,
                    "az_editor_ui::panels::ViewportPanel::NAME"
                )
                && az_architecture_guard::symbols_contain(
                    layout,
                    "pub const fn visual_graph_workbench()"
                )
                && az_architecture_guard::symbols_contain(layout, "WorkbenchKind::VisualGraph")
                && az_architecture_guard::symbols_contain(
                    layout,
                    "az_editor_ui::panels::VisualGraphPanel::NAME"
                )
                && !az_architecture_guard::symbols_contain(
                    layout,
                    "az_editor_ui::panels::ProjectWorkflowPanel::NAME"
                ),
            "workspace layout profile must default to viewport, keep graph as an explicit workbench, keep entity navigation/details in side docks, and put asset browser plus console in the bottom dock"
        );
    }

    /// Scene-profile dock membership, asserted against the live profile data
    /// the workspace consumes rather than against source text.
    fn assert_scene_profile_dock_membership() {
        // Dock membership is asserted against the live profile data (not
        // source text) so panel reordering or constant moves never break the
        // guard: `layout_profile_for_mode("scene")` is the same data the
        // workspace consumes.
        let scene_profile = crate::workspace::layout::layout_profile_for_mode("scene");
        let panel_names = |placements: &[crate::workspace::layout::PanelPlacement]| {
            placements
                .iter()
                .map(|placement| placement.panel_name)
                .collect::<Vec<_>>()
        };
        let left_panels = panel_names(scene_profile.left);
        let right_panels = panel_names(scene_profile.right);
        let bottom_panels = panel_names(scene_profile.bottom);
        assert!(
            left_panels.contains(&az_editor_ui::panels::AuthoredOutliner::NAME)
                && !left_panels.contains(&az_editor_ui::panels::AssetBrowser::NAME)
                && !left_panels.contains(&az_editor_ui::panels::Console::NAME),
            "O3DE-style loaded workspace left dock must be entity/outliner navigation, not asset browser or console"
        );
        assert!(
            right_panels.contains(&az_editor_ui::panels::AuthoredInspector::NAME)
                && !right_panels.contains(&az_editor_ui::panels::AssetBrowser::NAME)
                && !right_panels.contains(&az_editor_ui::panels::Console::NAME),
            "O3DE-style loaded workspace right dock must be the entity/details inspector"
        );
        assert!(
            bottom_panels.contains(&az_editor_ui::panels::AssetBrowser::NAME)
                && bottom_panels.contains(&az_editor_ui::panels::Console::NAME)
                && !bottom_panels.contains(&az_editor_ui::panels::AuthoredOutliner::NAME)
                && !bottom_panels.contains(&az_editor_ui::panels::AuthoredInspector::NAME),
            "O3DE-style loaded workspace bottom dock must contain Asset Browser and Console tabs"
        );
    }

    /// The adopted Aether workspace keeps its regions, its dock toggles, and
    /// the graph workbench, and mode profiles can still remove a dock.
    ///
    /// The empty-side needle reads `if spec.placements.is_empty()` rather than
    /// the old per-side `if bottom.is_empty()`: the three duplicated
    /// install-or-remove blocks were folded into one `apply_side_dock` helper
    /// keyed on [`SideDock`]. The guarded property is unchanged — an empty side
    /// still removes that dock, `remove_bottom_dock` included — only the
    /// spelling moved from three copies to one.
    fn assert_adopted_workspace_regions(sources: &EditorShellSources) {
        let app = &sources.app;
        let dock = &sources.dock;
        let layout = &sources.layout;
        assert!(
            az_architecture_guard::symbols_contain(
                dock,
                "set_bottom_dock_extent(BottomDockExtent::BoundedByRight"
            ) && az_architecture_guard::symbols_contain(dock, "remove_bottom_dock")
                && az_architecture_guard::symbols_contain(dock, "if spec.placements.is_empty()"),
            "loaded workspace dock must bound bottom by right inspector, and mode profiles must be able to remove docks (materials/scripting/gamedata have no bottom)"
        );
        assert!(
            az_architecture_guard::symbols_contain(
                app,
                "pub use aether_editor_view::AetherEditorView as EditorView"
            ) && az_architecture_guard::symbols_contain(app, "pub struct AetherEditorView")
                && az_architecture_guard::symbols_contain(app, "create_default_dock_area")
                && az_architecture_guard::source_literal_contains(
                    app,
                    "\"aether-workspace-dock-area\""
                )
                && az_architecture_guard::symbols_contain(app, "apply_workspace_layout_for_mode(")
                && az_architecture_guard::symbols_contain(app, "set_mode_with_window")
                && az_architecture_guard::symbols_contain(dock, "apply_workspace_layout")
                && az_architecture_guard::symbols_contain(layout, "AnimationMotionTrackPanel")
                && az_architecture_guard::symbols_contain(layout, "WorkbenchKind::Animation")
                && az_architecture_guard::symbols_contain(layout, "WorkbenchKind::Material"),
            "attached project launch must use the adopted Aether editor workspace as the live EditorView with the profile-driven DockArea always mounted"
        );
        let sym =
            |source: &str, snippet: &str| az_architecture_guard::symbols_contain(source, snippet);
        assert!(
            sym(app, "struct AetherTitlebarRegion")
                && sym(app, "struct AetherToolbarRegion")
                && sym(app, "struct AetherWorkspaceRegion")
                && sym(app, "struct AetherStatusRegion")
                && sym(app, "struct AetherModalRegion")
                && sym(app, "toggle_left_panel_visibility")
                && sym(app, "toggle_right_panel_visibility")
                && sym(app, "toggle_bottom_panel_visibility")
                && sym(app, "set_scene_dock_open")
                && sym(app, "open_center_workbench")
                && sym(app, "visual_graph_workbench")
                && sym(app, "DockPlacement::Left")
                && sym(app, "DockPlacement::Right")
                && sym(app, "DockPlacement::Bottom")
                && sym(app, "toggle_left_dock")
                && sym(app, "toggle_asset_browser_dock")
                && sym(app, "toggle_right_dock")
                && sym(app, "toggle_bottom_console_dock")
                && sym(app, "toggle_bottom_session_dock")
                && sym(app, "show_graph_workspace")
                && sym(app, "ToggleGraphPanel")
                && !sym(app, "ToggleProjectWorkflowPanel")
                && !sym(app, "toggle_bottom_project_workflow_dock"),
            "adopted editor workspace must retain titlebar, status regions, DockArea toggles, and graph workbench open without reintroducing project workflow as a dock panel"
        );
    }

    /// The menu and the default keymap reach the graph workbench without
    /// reintroducing project setup as a docked panel.
    fn assert_menu_and_keymap_expose_the_graph_workbench(sources: &EditorShellSources) {
        let menu = &sources.menu;
        let keymaps = &sources.keymaps;
        let sym =
            |source: &str, snippet: &str| az_architecture_guard::symbols_contain(source, snippet);
        assert!(
            az_architecture_guard::source_literal_contains(menu, "Visual Graph Workbench")
                && !az_architecture_guard::source_literal_contains(menu, "Project Setup")
                && !sym(menu, "ToggleProjectWorkflowPanel"),
            "loaded editor menu must not expose project setup as a docked workspace panel"
        );
        assert!(
            az_architecture_guard::source_literal_contains(keymaps, "ctrl-5")
                && sym(keymaps, "crate::actions::ToggleGraphPanel"),
            "default keymap must expose the visual graph workspace for dogfooding"
        );
    }

    #[test]
    fn editor_attach_stays_on_daemon_and_session_supervisor_discovery() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("attach.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let production_source = production_source_without_cfg_test_modules(&source);

        for required in [
            "AzDaemonClient::connect",
            "register_project_root",
            "resolve_session_supervisor_descriptor",
            "list_session_supervisor_descriptors",
            "session_manifest",
            "SessionSupervisorClient::connect_for_session",
            "resolve_project_host_descriptor",
            "resolve_asset_processor_descriptor",
            "ProjectHostClient::connect_for_session",
            "AssetProcessorClient::connect_for_session",
            "workspace_snapshot",
            "type_registry_snapshot",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&production_source, required),
                "editor attach must keep daemon/session-supervisor IPC discovery step `{required}`"
            );
        }

        for &(forbidden, _) in ATTACH_DISCOVERY_BYPASSES {
            assert!(
                !az_architecture_guard::symbols_contain(&production_source, forbidden),
                "editor attach must not bypass daemon/session-supervisor discovery with `{forbidden}`"
            );
        }
    }

    /// Positive control for the three guards above: every needle fires on the
    /// bypass it forbids, and none of them matches the discovery steps the same
    /// guards require.
    #[test]
    fn attach_and_console_needles_fire_and_spare_the_discovery_shapes() {
        const DISCOVERY_SHAPES: &[&str] = &[
            "let supervisor = SessionSupervisorClient::connect_for_session(&descriptor, &session_id).await?;",
            "let descriptor = resolve_asset_processor_descriptor(&supervisor, &session_slug).await?;",
            "let output = supervisor.exec_command(&session_id, line).await?;",
            "EditorApp::open_project_session_via_daemon(project_root, cx)?;",
        ];

        for &(needle, reintroduction) in ATTACH_DISCOVERY_BYPASSES
            .iter()
            .chain(CONSOLE_PROCESS_BYPASSES)
            .chain(STANDALONE_LOCAL_ATTACH_FALLBACKS)
        {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the bypass it forbids: {reintroduction}"
            );
            for shape in DISCOVERY_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the discovery shape `{shape}`"
                );
            }
        }
    }

    #[test]
    fn editor_runtime_host_uses_shared_side_channel_readers() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("runtime_host.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        assert!(
            az_architecture_guard::symbols_contain(
                &source,
                "validate_side_channel_capability_matches("
            ) && az_architecture_guard::symbols_contain(&source, "SideChannelKind::GpuSurface")
                && az_architecture_guard::symbols_contain(&source, "publish_frame_source("),
            "editor runtime-host must validate brokered GPU viewport side-channel handles without CPU pixel-file fallbacks"
        );
        for &(forbidden, _) in RUNTIME_HOST_SIDE_CHANNEL_FILE_READERS {
            assert!(
                !az_architecture_guard::symbols_contain(&source, forbidden),
                "editor runtime-host must not own side-channel file validation/reading: `{forbidden}`"
            );
        }
    }

    /// Positive control for the guard above: every needle fires on the CPU
    /// pixel-file fallback it forbids, and none of them matches the brokered
    /// GPU side-channel shapes the same guard requires.
    #[test]
    fn runtime_host_side_channel_needles_fire_and_spare_the_brokered_shapes() {
        const BROKERED_SHAPES: &[&str] = &[
            "validate_side_channel_capability_matches(&handle, SideChannelKind::GpuSurface)?;",
            "publish_frame_source(&handle, cx);",
        ];

        for &(needle, reintroduction) in RUNTIME_HOST_SIDE_CHANNEL_FILE_READERS {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the file fallback it forbids: {reintroduction}"
            );
            for shape in BROKERED_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the brokered side-channel shape `{shape}`"
                );
            }
        }
    }

    #[test]
    fn aether_style_does_not_install_implicit_hover_state() {
        let model_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app")
            .join("aether_editor_model.rs");
        let model_source = std::fs::read_to_string(&model_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", model_path.display()));
        let view_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app")
            .join("aether_editor_view.rs");
        let view_source = std::fs::read_to_string(&view_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", view_path.display()));
        let render_source = aether_render_sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src")
                .join("app"),
        );

        for &(forbidden, _) in AETHER_IMPLICIT_HOVER_STYLE_KEYS {
            assert!(
                !az_architecture_guard::source_literal_contains(&model_source, forbidden),
                "AetherStyle must remain base style only; GPUI .hover owns hover state, not `{forbidden}`"
            );
            assert!(
                !az_architecture_guard::source_literal_contains(&view_source, forbidden),
                "AetherStyle adapter must not infer GPUI hover state from `{forbidden}`"
            );
            assert!(
                !az_architecture_guard::source_literal_contains(&render_source, forbidden),
                "Aether renderer must not infer GPUI hover state from `{forbidden}`"
            );
        }
    }

    /// Positive control for the guard above: every key fires on the style
    /// lookup it forbids, and none of them matches GPUI's own hover call.
    ///
    /// `AetherStyle` is a `BTreeMap<String, String>` read through
    /// `style.get("key")`, so these keys only ever exist inside string
    /// literals. As `symbols_contain` needles they could never fire — literal
    /// contents are stripped — so they are checked as contract text (ticket
    /// 042).
    #[test]
    fn aether_hover_style_keys_fire_and_spare_the_gpui_hover_call() {
        const GPUI_HOVER_CALL: &str = "let row = row.hover(|style| style.bg(theme.accent));";

        for &(key, reintroduction) in AETHER_IMPLICIT_HOVER_STYLE_KEYS {
            assert!(
                az_architecture_guard::source_literal_contains(reintroduction, key),
                "key `{key}` cannot match the style lookup it forbids: {reintroduction}"
            );
            assert!(
                !az_architecture_guard::source_literal_contains(GPUI_HOVER_CALL, key),
                "key `{key}` rejects GPUI's own hover call `{GPUI_HOVER_CALL}`"
            );
        }
    }

    #[test]
    fn aether_render_implementation_is_view_owned_and_uses_state_projections() {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app");
        let model =
            std::fs::read_to_string(app_dir.join("aether_editor_model.rs")).expect("read model");
        let render = aether_render_sources(&app_dir);

        for symbol in [
            "impl AetherEditorView",
            "struct SettingOption",
            "struct SettingsRowProjection",
            "struct SourceControlSegment",
            "struct SelectionFileTypeProjection",
            "struct AssetPipelineCounts",
        ] {
            assert!(
                !az_architecture_guard::symbols_contain(&model, symbol),
                "Aether model must not retain renderer symbol `{symbol}`"
            );
            assert!(
                az_architecture_guard::symbols_contain(&render, symbol),
                "Aether renderer must own `{symbol}`"
            );
        }

        for &(raw_child, _) in AETHER_RAW_STATE_CHILDREN {
            assert!(
                !az_architecture_guard::symbols_contain(&render, raw_child),
                "Aether renderer must consume a state projection or command, not `{raw_child}`"
            );
        }
    }

    #[test]
    fn aether_view_render_domains_stay_bounded_and_explicit() {
        let app_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app");
        let render_dir = app_dir.join("aether_editor_view_render");
        let composer_path = render_dir.join("mod.rs");
        let composer = std::fs::read_to_string(&composer_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", composer_path.display()));

        assert!(
            composer.lines().count() <= 32,
            "{} must stay a short domain composer, not a renderer dependency bag",
            composer_path.display()
        );
        for module in [
            "asset_browser",
            "authored_content",
            "diagnostics_preview",
            "gamedata_gem",
            "presentation",
            "workspace_overlay",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&composer, &format!("mod {module};")),
                "{} must compose the `{module}` view domain",
                composer_path.display()
            );
        }

        for path in aether_render_source_files(&render_dir) {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert!(
                !source.contains("use super::*"),
                "{} must name the domain surface it consumes instead of importing a renderer prelude",
                path.display()
            );
        }

        for (relative, line_cap) in [
            ("asset_browser.rs", 1_800),
            ("authored_content/graphs_animation.rs", 1_000),
            ("authored_content/inspector.rs", 700),
            ("authored_content/scene_prefab.rs", 1_200),
            ("authored_content/schema_presentation.rs", 100),
            ("diagnostics_preview.rs", 1_500),
            ("gamedata_gem.rs", 500),
            ("presentation.rs", 400),
            ("workspace_overlay/dock.rs", 500),
            ("workspace_overlay/menus_settings.rs", 1_200),
            ("workspace_overlay/shell.rs", 1_600),
        ] {
            let path = render_dir.join(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert!(
                source.lines().count() <= line_cap,
                "{} must remain a reviewable domain module, not a replacement renderer monolith",
                path.display()
            );
        }

        for retired_flat_module in [
            app_dir.join("aether_editor_view_render.rs"),
            render_dir.join("workspace_overlay.rs"),
            render_dir.join("authored_content.rs"),
            render_dir.join("helpers.rs"),
        ] {
            assert!(
                !retired_flat_module.exists(),
                "{} must not survive as a compatibility or catch-all renderer module",
                retired_flat_module.display()
            );
        }
    }

    #[test]
    fn aether_authored_content_state_is_presentation_only() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app")
            .join("aether_authored_content_state.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        for required in [
            "struct AetherAuthoredContentState",
            "struct AetherAuthoredExpansion",
            "struct AetherAuthoredSourcePaths",
            "fn set_expanded",
            "fn record_source_path_move",
            "fn clear_resolved_source_path_moves",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&source, required),
                "Aether authored-content presentation state must retain `{required}`"
            );
        }
        for &(forbidden, _) in AETHER_AUTHORED_CONTENT_AUTHORITY {
            assert!(
                !az_architecture_guard::symbols_contain(&source, forbidden),
                "Aether authored-content presentation state must not own selection, mutation, or RPC authority: `{forbidden}`"
            );
        }
    }

    #[test]
    fn aether_diagnostics_state_has_no_runtime_or_mannequin_mirror() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("app")
            .join("aether_diagnostics_state.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

        for required in [
            "struct AetherDiagnosticsState",
            "struct AetherConsolePresentation",
            "fn select_console_filter",
            "fn set_console_query",
            "fn toggle_stats",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&source, required),
                "Aether diagnostics presentation state must retain `{required}`"
            );
        }
        for &(forbidden, _) in AETHER_DIAGNOSTICS_CONTROLLER_MIRRORS {
            assert!(
                !az_architecture_guard::symbols_contain(&source, forbidden),
                "Aether diagnostics state must not mirror controller truth or retain background/client state: `{forbidden}`"
            );
        }
    }

    /// Positive control for the three Aether-state guards above: every needle
    /// fires on the raw child, authority, or controller mirror it forbids, and
    /// none of them matches the presentation-only shapes those states keep.
    #[test]
    fn aether_state_needles_fire_and_spare_the_presentation_shapes() {
        const PRESENTATION_SHAPES: &[&str] = &[
            "let workspace = self.state.workspace_projection(cx);",
            "pub struct AetherAuthoredExpansion { pub expanded: BTreeSet<String> }",
            "fn record_source_path_move(&mut self, from: String, to: String) {}",
            "pub struct AetherConsolePresentation { pub query: String }",
            "fn select_console_filter(&mut self, filter: ConsoleFilter) {}",
        ];

        for &(needle, reintroduction) in AETHER_RAW_STATE_CHILDREN
            .iter()
            .chain(AETHER_AUTHORED_CONTENT_AUTHORITY)
            .chain(AETHER_DIAGNOSTICS_CONTROLLER_MIRRORS)
        {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the shape it forbids: {reintroduction}"
            );
            for shape in PRESENTATION_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the presentation-only shape `{shape}`"
                );
            }
        }
    }

    #[test]
    fn mode_projections_have_one_typed_lifecycle_and_no_reverse_republish_edges() {
        let source_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let framework = std::fs::read_to_string(source_dir.join("mode_projection.rs"))
            .expect("read mode projection framework");
        for required in [
            "trait ModeProjectionSpec",
            "struct ModeProjectionRegistry",
            "fn install_mode_projections",
            "fn publish_mode_projection",
            "observe_global",
        ] {
            assert!(
                az_architecture_guard::symbols_contain(&framework, required),
                "mode projection framework must own `{required}`"
            );
        }

        for mode_file in ["materials_ui.rs", "scripting_ui.rs", "game_data_catalog.rs"] {
            let source = std::fs::read_to_string(source_dir.join(mode_file))
                .unwrap_or_else(|error| panic!("read {mode_file}: {error}"));
            assert!(
                az_architecture_guard::symbols_contain(&source, "impl ModeProjectionSpec"),
                "{mode_file} must register through ModeProjectionSpec"
            );
            for &(retired, _) in RETIRED_MODE_PROJECTION_PUBLISHERS {
                assert!(
                    !az_architecture_guard::symbols_contain(&source, retired),
                    "{mode_file} must not restore retired publisher `{retired}`"
                );
            }
        }

        for upstream_file in [
            "authored_selection.rs",
            "graph_ui.rs",
            "project_workflow.rs",
            "asset_processor/mod.rs",
        ] {
            let source = std::fs::read_to_string(source_dir.join(upstream_file))
                .unwrap_or_else(|error| panic!("read {upstream_file}: {error}"));
            for &(forbidden, _) in RETIRED_MODE_PROJECTION_REPUBLISH_EDGES {
                assert!(
                    !az_architecture_guard::symbols_contain(&source, forbidden),
                    "{upstream_file} must publish its typed global, not call `{forbidden}`"
                );
            }
        }
    }

    /// Positive control for the guard above: every needle fires on the retired
    /// publisher or reverse republish edge it forbids, and none of them matches
    /// the typed `ModeProjectionSpec` registration that replaced them.
    #[test]
    fn mode_projection_needles_fire_and_spare_the_registered_specs() {
        const REGISTERED_SPEC_SHAPES: &[&str] = &[
            "crate::materials_ui::mode_projection_registration()?,",
            "crate::scripting_ui::mode_projection_registration()?,",
            "crate::game_data_catalog::mode_projection_registration()?,",
            "impl ModeProjectionSpec for MaterialsUiProjection {}",
        ];

        for &(needle, reintroduction) in RETIRED_MODE_PROJECTION_PUBLISHERS
            .iter()
            .chain(RETIRED_MODE_PROJECTION_REPUBLISH_EDGES)
        {
            assert!(
                az_architecture_guard::symbols_contain(reintroduction, needle),
                "needle `{needle}` cannot match the publisher it forbids: {reintroduction}"
            );
            for shape in REGISTERED_SPEC_SHAPES {
                assert!(
                    !az_architecture_guard::symbols_contain(shape, needle),
                    "needle `{needle}` rejects the registered projection spec `{shape}`"
                );
            }
        }
    }

    #[test]
    fn production_source_scanner_ignores_cfg_test_modules_only() {
        let source = r"
pub fn production_path() {}
#[cfg(test)]
mod tests {
    fn fixture() {
        let _ = ProjectHostRpc::new(ProjectHost::new());
    }
}
";
        let production_source = production_source_without_cfg_test_modules(source);

        assert!(az_architecture_guard::symbols_contain(
            &production_source,
            "production_path"
        ));
        assert!(!az_architecture_guard::symbols_contain(
            &production_source,
            "ProjectHostRpc::new("
        ));
    }

    #[test]
    fn production_source_scanner_recurses_into_editor_binary() {
        let paths = editor_production_sources();

        assert!(
            paths
                .iter()
                .any(|source| source.path.ends_with("src/bin/az-editor.rs")),
            "production editor boundary guard must scan the standalone editor binary"
        );
    }

    fn assert_pub_function_is_cfg_test(source: &str, function: &str, path: &std::path::Path) {
        assert!(
            public_functions_have_cfg(source, function, &["#[cfg(test)]"]),
            "{} {function} must remain test-only; production editor code must construct service clients from descriptors/endpoints and route saves through SessionSupervisor.saveDocument",
            path.display()
        );
    }

    fn editor_production_sources() -> Vec<az_architecture_guard::ProductionSource> {
        let source_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        collect_production_rust_sources(source_dir).expect("collect az-editor production sources")
    }

    fn forbidden_editor_dependencies(manifest: &Value, workspace_manifest: &Value) -> Vec<String> {
        let mut exact_names = Vec::new();
        exact_names.extend_from_slice(FORBIDDEN_PRODUCTION_DEPS);
        exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
        let mut prefixes = vec!["az-framework-"];
        prefixes.extend_from_slice(PROJECT_OR_GAME_DEPENDENCY_PREFIXES);
        forbidden_production_dependencies(
            manifest,
            workspace_manifest,
            DependencyBoundary::new(&exact_names, &prefixes, PROJECT_OR_FORMAT_PATH_SEGMENTS)
                // `az-gem-contract` is the composition vocabulary, not a gem —
                // the `az-gem-` prefix catches it by spelling alone, which is
                // the same false positive project-host's guard already exempts.
                .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES),
        )
    }
}
