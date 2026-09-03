//! UI components for the `AZoth` Editor.
//!
//! This crate provides reusable UI components built on top of GPUI,
//! following the `AZoth` Editor design system and styling conventions.

pub mod error;
pub mod fuzzy;
pub mod gems;
pub mod panels;
pub mod project_build;
pub mod scene_tools;
pub mod status;
pub mod theme;
pub mod type_iconography;
pub mod viewport_texture;

pub use error::{Error, Result};
pub use gems::{
    EditorGemCatalog, EditorGemCreationStatus, EditorGemDependencyInfo, EditorGemDeprecationInfo,
    EditorGemInfo, EditorGemRebuildState, EditorGemReplacementInfo, EditorGemSelection,
    NewGemCapabilityChoice, derived_gem_id, is_valid_gem_name,
};
pub mod actions;
pub mod menu;
pub mod naming;
pub mod settings;

// Re-export theme types and functions
pub use theme::{ActiveTheme, Theme, ThemeRegistry};

// Re-export panel types
pub use panels::*;
pub use project_build::*;
pub use scene_tools::*;
pub use status::{ServiceHealthStateData, StatusTone};

#[cfg(test)]
mod architecture_tests {
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, PROJECT_OR_FORMAT_PATH_SEGMENTS,
        PROJECT_OR_GAME_DEPENDENCY_PREFIXES, collect_production_rust_sources,
        forbidden_production_dependencies,
    };

    use toml::Value;

    const FORBIDDEN_UI_DEPS: &[&str] = &[
        "az-asset",
        "az-asset-builder",
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        "az-engine",
        "az-framework",
        "az-observability",
        "az-project",
        "az-project-host",
        "az-proto-asset",
        "az-proto-core",
        "az-proto-daemon",
        "az-proto-observability",
        // az-proto-project is exempted below: ADR 0022 freezes the vNext
        // reflection/command vocabulary inside it as a pure data contract
        // (`vnext` — no Cap'n Proto bindings), which the ui crate consumes as
        // its view model. Relocating that module was attempted (014) and is
        // blocked by the wire encodings' inherent impls. The boundary this
        // guard protects is *transport and service hosting*, so the wire half
        // stays forbidden via `FORBIDDEN_UI_SYMBOLS` in
        // `ui_sources_stay_off_wire_and_transport`.
        "az-proto-runtime",
        "az-proto-session",
        "az-rpc",
        "az-runtime-host",
        "az-session",
        "az-sessiond",
        "bevy",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "sample-plugin",
    ];

    /// Every forbidden-needle table below pairs each needle with the
    /// reintroduction it must fire on, and the controls iterate the table. The
    /// samples are written out rather than spliced together from the needle:
    /// ticket 018's `use az_proto_` prefix matches `format!("{needle}::Foo;")`
    /// but nothing a person would ever write, so an interpolated sample proves
    /// only that the interpolation worked (ticket 042).
    ///
    /// Wire/transport symbols the ui crate must never name, even though the
    /// pure-data half of `az-proto-project` (`vnext`) is its sanctioned view
    /// model. If a panel starts encoding Cap'n Proto payloads or reaching for
    /// transport handles, it has stopped being a renderer of editor-owned
    /// data.
    const FORBIDDEN_UI_SYMBOLS: &[(&str, &str)] = &[
        (
            "vnext_wire",
            "use az_proto_project::vnext_wire::encode_reflected_value;",
        ),
        (
            "project_capnp",
            "use az_proto_project::project_capnp::project_host;",
        ),
        (
            "capnp::",
            "let reader: capnp::message::Reader<_> = payload.into_reader();",
        ),
        ("capnp_rpc", "use capnp_rpc::RpcSystem;"),
        (
            "Endpoint::new",
            "let endpoint = Endpoint::new(kind, address);",
        ),
        (
            "connect_twoparty_bootstrap",
            "let client = connect_twoparty_bootstrap(endpoint).await?;",
        ),
        (
            "tokio::",
            "tokio::spawn(async move { refresh_panel(entity).await });",
        ),
    ];

    /// The sanctioned view-model shapes: the pure-data `vnext` contract the ui
    /// crate is allowed to name. No wire/transport or protocol-import needle
    /// may match one of these, or the guard would forbid its own view model.
    const SANCTIONED_VIEW_MODEL_SHAPES: &[&str] = &[
        "use az_proto_project::vnext::ReflectedTypeKind;",
        "use az_proto_project::vnext::{PrefabEditCommand, ReflectedValue};",
        "let kind = az_proto_project::vnext::ReflectedTypeKind::Struct;",
    ];

    /// State→label and state→tone tables retired into `status.rs` (ticket 019).
    /// A panel that grows its own copy is how the vocabulary drifted into five
    /// spellings of "succeeded"; these names may only exist inside the shared
    /// module, and callers reach the words through the enums' own methods.
    const RETIRED_UI_STATUS_TABLES: &[(&str, &str)] = &[
        (
            "asset_job_status_label",
            "fn asset_job_status_label(status: AssetBrowserJobStatus) -> &'static str { \"Queued\" }",
        ),
        (
            "asset_status_color",
            "fn asset_status_color(status: AssetBrowserEntryStatus, theme: &Theme) -> Hsla { theme.accent }",
        ),
        (
            "asset_status_label",
            "fn asset_status_label(status: AssetBrowserEntryStatus) -> &'static str { \"Stale\" }",
        ),
        (
            "process_state_label",
            "fn process_state_label(state: SessionProcessStateData) -> &'static str { \"Running\" }",
        ),
        (
            "runtime_state_label",
            "fn runtime_state_label(state: EditorRuntimeStateData) -> &'static str { \"Stopped\" }",
        ),
        (
            "runtime_status_text_color",
            "fn runtime_status_text_color(state: EditorRuntimeStateData, theme: &Theme) -> Hsla { theme.danger }",
        ),
        (
            "service_role_label",
            "fn service_role_label(role: SessionServiceRoleData) -> &'static str { \"Project Host\" }",
        ),
        (
            "session_state_label",
            "fn session_state_label(state: EditorSessionStateData) -> &'static str { \"Active\" }",
        ),
    ];

    /// The shared module must actually own the tables it took over.
    const STATUS_MODULE_OWNED_TABLES: &[&str] = &[
        "impl AssetBrowserEntryStatus",
        "impl AssetBrowserJobStatus",
        "impl EditorProjectBuildPhase",
        "impl EditorRuntimeStateData",
        "impl EditorSessionStateData",
        "impl GraphBuildJobStatusData",
        "impl GraphBuildSourceStatusData",
        "impl ServiceHealthStateData",
        "impl SessionProcessStateData",
        "impl SessionServiceRoleData",
        "impl StatusTone",
    ];

    /// Outliner filtering runs once per keystroke and once per repaint over the
    /// whole authored tree (ticket 021). These are the shapes that made it
    /// allocate: a lowercased copy of every inspected string, and a deep clone
    /// of a global the panel only reads. Third column is the reintroduction
    /// each needle must still fire on.
    const OUTLINER_PER_REPAINT_ALLOCATIONS: &[(&str, &str, &str)] = &[
        (
            "to_ascii_lowercase",
            "lowercase allocation per inspected string",
            "field.name.to_ascii_lowercase().contains(filter)",
        ),
        (
            "to_lowercase",
            "lowercase allocation per inspected string",
            "let filter = filter.to_lowercase();",
        ),
        (
            "try_global::<EditorAuthoredOutline>().cloned()",
            "deep clone of the authored outline tree",
            "let outline = cx.try_global::<EditorAuthoredOutline>().cloned();",
        ),
        (
            "and_then(EditorReflectedSelectionState::current).cloned()",
            "deep clone of the reflected inspection",
            "cx.try_global::<EditorReflectedSelectionState>().and_then(EditorReflectedSelectionState::current).cloned()",
        ),
    ];

    /// The allocation-free shapes that replaced them. No needle above may match
    /// one of these, or the guard would forbid its own fix.
    const OUTLINER_BORROWED_SHAPES: &[&str] = &[
        "contains_ignore_ascii_case(&field.name, filter)",
        "let outline = cx.try_global::<EditorAuthoredOutline>();",
        "cx.try_global::<EditorReflectedSelectionState>().and_then(EditorReflectedSelectionState::current)",
    ];

    /// The graph panel rebuilds its whole element tree on every pointer event of
    /// a node drag, an anchor drag, or a canvas pan (ticket 022). These are the
    /// shapes that made a repaint allocate: deep clones of the projection global
    /// and of the theme, and a context menu rebuilt whether or not it is open.
    /// Third column is the reintroduction each needle must still fire on.
    const GRAPH_PER_REPAINT_ALLOCATIONS: &[(&str, &str, &str)] = &[
        (
            "try_global::<EditorGraphDocumentProjection>().cloned()",
            "deep clone of the whole graph projection",
            "let projection = cx.try_global::<EditorGraphDocumentProjection>().cloned().unwrap_or_else(EditorGraphDocumentProjection::empty);",
        ),
        (
            "cx.theme().clone()",
            "deep clone of the theme",
            "let theme = cx.theme().clone();",
        ),
        (
            "let menu_items = graph_context_menu_items(&projection.node_palette)",
            "context-menu items rebuilt every repaint instead of at open time",
            "let menu_items = graph_context_menu_items(&projection.node_palette);",
        ),
    ];

    /// The borrowed shapes that replaced them. No needle above may match one of
    /// these, or the guard would forbid its own fix.
    const GRAPH_BORROWED_SHAPES: &[&str] = &[
        "let projection = cx.try_global::<EditorGraphDocumentProjection>().unwrap_or(&NO_GRAPH_PROJECTION);",
        "let theme = cx.theme();",
        "graph_context_menu_items(&projection.node_palette),",
    ];

    /// [`az_architecture_guard::symbol_skeleton`] strips literal contents, so the
    /// per-anchor element id has to be checked as contract text instead.
    const GRAPH_ROUTE_ANCHOR_FORMAT_ID: &str = "\"graph-route-anchor-{}-{}\"";

    // `az_proto_project` is absent by design; see the `FORBIDDEN_UI_DEPS` rationale.
    const FORBIDDEN_UI_PROTO_IMPORTS: &[(&str, &str, &str)] = &[
        (
            "use az_proto_asset",
            "asset protocol crate import",
            "use az_proto_asset::asset_capnp::asset_record;",
        ),
        (
            "use az_proto_core",
            "core protocol crate import",
            "use az_proto_core::ServiceDescriptor;",
        ),
        (
            "use az_proto_daemon",
            "daemon protocol crate import",
            "use az_proto_daemon::daemon_capnp::az_daemon;",
        ),
        (
            "use az_proto_observability",
            "observability protocol crate import",
            "use az_proto_observability::observability_capnp::log_event;",
        ),
        (
            "use az_proto_runtime",
            "runtime protocol crate import",
            "use az_proto_runtime::runtime_capnp::runtime_host;",
        ),
        (
            "use az_proto_session",
            "session protocol crate import",
            "use az_proto_session::SessionState;",
        ),
    ];

    /// Service, protocol, database, process, and webview boundaries the ui
    /// crate must not open. Third column is the reintroduction each needle
    /// must still fire on.
    const FORBIDDEN_UI_SOURCE_BOUNDARIES: &[(&str, &str, &str)] = &[
        (
            "use az_project",
            "project workflow crate import",
            "use az_project::ProjectManifest;",
        ),
        (
            "use az_session",
            "session workflow crate import",
            "use az_session::SessionManager;",
        ),
        (
            "use az_daemon",
            "daemon crate import",
            "use az_daemon::AzDaemon;",
        ),
        (
            "use az_rpc",
            "RPC transport crate import",
            "use az_rpc::Endpoint;",
        ),
        (
            "use az_asset_processor",
            "asset-processor service crate import",
            "use az_asset_processor::AssetProcessor;",
        ),
        (
            "use az_project_host",
            "project-host service crate import",
            "use az_project_host::ProjectHost;",
        ),
        (
            "use az_runtime_host",
            "runtime-host service crate import",
            "use az_runtime_host::RuntimeHost;",
        ),
        (
            "capnp::",
            "raw Cap'n Proto API",
            "let root = payload.get_root::<capnp::any_pointer::Reader>()?;",
        ),
        (
            "az_rpc::",
            "RPC transport API",
            "let endpoint = az_rpc::Endpoint::tcp(address);",
        ),
        ("drizzle", "DB adapter API", "use drizzle::Drizzle;"),
        (
            "turso",
            "DB engine API",
            "let connection = turso::Connection::open(path)?;",
        ),
        ("rusqlite", "DB engine API", "use rusqlite::Connection;"),
        (
            "std::process::",
            "process spawning API",
            "std::process::Command::new(\"az\").spawn()?;",
        ),
        (
            "use std::process",
            "process spawning import",
            "use std::process::Command;",
        ),
        (
            "process::Command",
            "process spawning API",
            "let child = process::Command::new(\"az\").spawn()?;",
        ),
        (
            "webview",
            "webview backend",
            "let webview = WebViewBuilder::new(&window).build()?;",
        ),
        (
            "wry::",
            "webview backend",
            "let builder = wry::WebViewBuilder::new(&window);",
        ),
        (
            "tauri::",
            "webview backend",
            "tauri::Builder::default().run(context)?;",
        ),
        (
            "bevy::",
            "engine/runtime backend",
            "app.add_plugins(bevy::DefaultPlugins);",
        ),
    ];

    /// File access and serialization the settings module must not grow: the
    /// editor shell resolves paths, reads, and writes; `settings.rs` is the
    /// data shape and its GPUI global.
    const SETTINGS_FILE_AND_FORMAT_POLICY: &[(&str, &str)] = &[
        ("std::fs", "let text = std::fs::read_to_string(path)?;"),
        ("std::io", "use std::io::Write;"),
        ("std::path", "use std::path::PathBuf;"),
        ("PathBuf", "pub settings_file: PathBuf,"),
        (
            "directories::BaseDirs",
            "let base = directories::BaseDirs::new().expect(\"base dirs\");",
        ),
        (
            "serde_json",
            "let encoded = serde_json::to_string(&settings)?;",
        ),
        (
            "toml",
            "let settings = toml::from_str::<EditorSettings>(&text)?;",
        ),
        (
            "SettingsPaths",
            "pub struct SettingsPaths { pub file: PathBuf }",
        ),
        ("load_settings", "let settings = load_settings(&paths)?;"),
        ("save_settings", "save_settings(&paths, &settings)?;"),
    ];

    /// Host directory and file-watcher policy the theme modules must not own.
    ///
    /// The retired `ThemeLocation::User,` needle is gone (ticket 042): the
    /// variant is a tuple variant, so the identifier is never followed by a
    /// comma in real code and the needle could not fire, while the spelling
    /// that *can* fire (`ThemeLocation::User(`) matches `manager.rs`'s
    /// legitimate precedence match over shell-supplied locations. What the
    /// entry actually guarded — az-editor-ui resolving a user directory for
    /// itself — is covered by the `BaseDirs`/`current_exe`/`current_dir`
    /// needles here, by `home_dir` which replaces it, and by the `directories`
    /// and `notify` dependency half of the same test.
    const THEME_HOST_DIRECTORY_POLICY: &[(&str, &str)] = &[
        (
            "BaseDirs",
            "let dirs = BaseDirs::new().expect(\"base dirs\");",
        ),
        ("current_exe", "let exe = std::env::current_exe()?;"),
        ("current_dir", "let cwd = std::env::current_dir()?;"),
        (
            "home_dir",
            "let user_themes = std::env::home_dir()?.join(\".azoth/themes\");",
        ),
        ("watch_dir", "registry.watch_dir(&location, cx)?;"),
    ];

    /// Authored-edit actions retired by the native `PrefabEditCommand` payload.
    /// The guard checks `pub struct {name}`; the sample is the declaration it
    /// must fire on.
    const OBSOLETE_AUTHORED_EDIT_ACTIONS: &[(&str, &str)] = &[
        (
            "EditAuthoredPath",
            "pub struct EditAuthoredPath { pub path: String, pub value: String }",
        ),
        (
            "EditAuthoredPathText",
            "pub struct EditAuthoredPathText { pub path: String, pub text: String }",
        ),
        (
            "InitializeAuthoredPath",
            "pub struct InitializeAuthoredPath { pub path: String }",
        ),
        (
            "InsertAuthoredListItem",
            "pub struct InsertAuthoredListItem { pub path: String, pub index: usize }",
        ),
        (
            "MoveAuthoredListItem",
            "pub struct MoveAuthoredListItem { pub path: String, pub from: usize, pub to: usize }",
        ),
        (
            "InsertAuthoredMapEntry",
            "pub struct InsertAuthoredMapEntry { pub path: String, pub key: String }",
        ),
        (
            "ClearAuthoredPath",
            "pub struct ClearAuthoredPath { pub path: String }",
        ),
        (
            "RemoveAuthoredPath",
            "pub struct RemoveAuthoredPath { pub path: String }",
        ),
    ];

    /// Catalog placement fields no document-creation action may carry: the
    /// editor shell resolves placement from the project-host catalog.
    const CATALOG_PLACEMENT_FIELDS: &[(&str, &str)] = &[
        ("pub source_root:", "pub source_root: String,"),
        ("pub path_prefix:", "pub path_prefix: String,"),
        ("pub extension:", "pub extension: String,"),
    ];

    /// Graph model detail the ui action payloads must not carry; editor-core
    /// converts UI-normalized actions into validated graph commands.
    const ADD_GRAPH_NODE_MODEL_DETAIL: &[(&str, &str)] = &[
        (
            "input_values",
            "pub input_values: Vec<GraphInputValueData>,",
        ),
        ("NodePortId", "pub ports: Vec<NodePortId>,"),
    ];

    const MOVE_GRAPH_NODE_MODEL_DETAIL: &[(&str, &str)] = &[
        ("GraphNodeLayout", "pub layout: GraphNodeLayout,"),
        ("GraphCommand", "pub command: GraphCommand,"),
        (
            "input_values",
            "pub input_values: Vec<GraphInputValueData>,",
        ),
    ];

    const MOVE_GRAPH_ROUTE_ANCHOR_MODEL_DETAIL: &[(&str, &str)] = &[
        ("GraphConnectionRoute", "pub route: GraphConnectionRoute,"),
        ("GraphCommand", "pub command: GraphCommand,"),
        (
            "outgoing_segment",
            "pub outgoing_segment: GraphRouteSegment,",
        ),
        ("style", "pub style: GraphRouteStyle,"),
    ];

    const GRAPH_COMMENT_MODEL_DETAIL: &[(&str, &str)] = &[
        ("GraphCommand", "pub command: GraphCommand,"),
        ("GraphCommentBounds", "pub bounds: GraphCommentBounds,"),
        ("GraphCommentId", "pub comment_id: GraphCommentId,"),
    ];

    const CONNECT_GRAPH_PORTS_MODEL_DETAIL: &[(&str, &str)] = &[
        ("GraphConnection", "pub connection: GraphConnection,"),
        ("GraphPortRef", "pub from: GraphPortRef,"),
        ("GraphCommand", "pub command: GraphCommand,"),
        ("route", "pub route: Vec<GraphRoutePoint>,"),
    ];

    /// Document construction and source placement the graph panel must leave to
    /// the project-host catalog.
    const GRAPH_PANEL_DOCUMENT_CONSTRUCTION: &[(&str, &str)] = &[
        (
            "VisualGraphDocument::new",
            "let document = VisualGraphDocument::new(graph_type);",
        ),
        (
            "GraphSourceWorkflow",
            "let workflow = GraphSourceWorkflow::for_project(project_root);",
        ),
        ("DocumentId::new", "let id = DocumentId::new(&stem);"),
        ("GraphNode::new", "let node = GraphNode::new(descriptor);"),
    ];

    /// Source-document placement detail the authored outliner must not inspect.
    const OUTLINER_PLACEMENT_DETAIL: &[(&str, &str)] = &[
        (
            "OwnedSourceDocumentHint",
            "let hint: OwnedSourceDocumentHint = schema.document_hint().to_owned();",
        ),
        (
            "SourceDocumentHint",
            "fn document_hint(schema: &Schema) -> SourceDocumentHint { schema.hint() }",
        ),
        (
            "PROJECT_SOURCE_ROOT_DOCUMENT_HINT",
            "if hint.key == PROJECT_SOURCE_ROOT_DOCUMENT_HINT { root = hint.value.clone(); }",
        ),
        ("path_prefix", "let prefix = hint.path_prefix.clone();"),
        ("extension", "let extension = schema.extension.clone();"),
    ];

    /// The placement hint *key* the outliner must not read. It only ever
    /// appears inside a string literal, and [`az_architecture_guard::
    /// symbol_skeleton`] strips literal contents, so this is checked as
    /// contract text — spelled as a needle it could never fire (ticket 042).
    const OUTLINER_SOURCE_ROOT_HINT_KEY: &str = "project:source-root";

    /// Local filesystem navigation state the asset browser must not carry; the
    /// asset-processor workspace view owns source roots and entries.
    const ASSET_BROWSER_FILESYSTEM_NAVIGATION: &[(&str, &str)] = &[
        ("PathBuf", "pub current_directory: PathBuf,"),
        (
            "absolute_path",
            "let absolute_path = source_root.join(&entry.name);",
        ),
        ("current_dir", "let cwd = std::env::current_dir()?;"),
        (
            "navigate_to",
            "fn navigate_to(&mut self, directory: &Path) { self.cursor = directory.to_owned(); }",
        ),
        (
            "assets_path",
            "let assets_path = project_root.join(\"assets\");",
        ),
    ];

    /// The same boundary in user-facing words. These live inside string
    /// literals, which [`sym`] strips, so they are checked as contract text
    /// instead of as needles that could never fire (ticket 042).
    const ASSET_BROWSER_FILESYSTEM_LABELS: &[(&str, &str)] = &[
        (
            "current directory",
            "Label::new(\"Up one from the current directory\")",
        ),
        ("File browser", "Label::new(\"File browser\")"),
    ];

    #[test]
    fn ui_crate_stays_data_only_and_service_agnostic() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-editor-ui Cargo.toml");
        let manifest = toml::from_str::<Value>(&manifest).expect("parse az-editor-ui Cargo.toml");
        let workspace_manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
                .expect("read workspace Cargo.toml");
        let workspace_manifest =
            toml::from_str::<Value>(&workspace_manifest).expect("parse workspace Cargo.toml");
        let violations = forbidden_ui_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-editor-ui must remain GPUI view/data state only; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn ui_sources_stay_off_wire_and_transport() {
        // The sanctioned `az-proto-project` dependency is the pure-data
        // `vnext` contract only. These symbols are what "using the wire half"
        // looks like; none may appear in ui production sources.
        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("scan az-editor-ui sources");
        let mut violations = Vec::new();
        for source in &sources {
            for &(symbol, _) in FORBIDDEN_UI_SYMBOLS {
                // Symbol-level, per ticket 012: a panel that merely *mentions*
                // `tokio::` in a comment is not reaching for transport.
                if sym(&source.source, symbol) {
                    violations.push(format!(
                        "{} names `{symbol}`",
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
            "az-editor-ui renders data; wire encoding and transport stay in editor-core:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn ui_crate_rejects_forbidden_workspace_package_aliases() {
        let manifest = toml::from_str::<Value>(
            r"
            [dependencies]
            project-ui = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>(
            r#"
            [workspace.dependencies]
            project-ui = { package = "sample-plugin", version = "0.1" }
            "#,
        )
        .unwrap();
        let violations = forbidden_ui_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec!["dependencies.project-ui(workspace package sample-plugin)"]
        );
    }

    #[test]
    fn ui_crate_rejects_legacy_format_and_gem_package_aliases() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            chunk-format = { package = "cry-chunk-assets", version = "0.1" }
            object-format = { package = "az-objectstream", version = "0.1" }
            camera-gem = { package = "az-gem-camera", version = "0.1" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_ui_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "dependencies.camera-gem(package az-gem-camera)",
                "dependencies.chunk-format(package cry-chunk-assets)",
                "dependencies.object-format(package az-objectstream)",
            ]
        );
    }

    #[test]
    fn ui_crate_rejects_forbidden_path_only_dependencies() {
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
        let violations = forbidden_ui_dependencies(&manifest, &workspace_manifest);

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
    fn ui_source_does_not_open_service_protocol_db_process_or_webview_boundaries() {
        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("collect az-editor-ui production sources");
        let mut violations = Vec::new();

        for source in sources {
            for &(needle, reason, _) in FORBIDDEN_UI_SOURCE_BOUNDARIES
                .iter()
                .chain(FORBIDDEN_UI_PROTO_IMPORTS)
            {
                if sym(&source.source, needle) {
                    violations.push(format!(
                        "{} contains `{needle}` ({reason})",
                        source.path.display()
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "az-editor-ui must stay a GPUI renderer of editor-owned data; forbidden source boundary use:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the needles above: the list they replaced used a
    /// `use az_proto_` prefix that [`sym`] tokenization can never match, so the
    /// guard passed by being unable to fail.
    ///
    /// The samples come from the tables, not from `format!("{needle}::Foo;")`:
    /// that spelling matches the inert prefix too, so the control that caught
    /// nothing is the one this test replaces (ticket 042).
    #[test]
    fn forbidden_proto_import_needles_fire_and_spare_the_view_model() {
        for &(needle, _, reintroduction) in FORBIDDEN_UI_PROTO_IMPORTS
            .iter()
            .chain(FORBIDDEN_UI_SOURCE_BOUNDARIES)
        {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the boundary it forbids: {reintroduction}"
            );
            for sanctioned in SANCTIONED_VIEW_MODEL_SHAPES {
                assert!(
                    !sym(sanctioned, needle),
                    "needle `{needle}` rejects the sanctioned vnext view model `{sanctioned}`"
                );
            }
        }
    }

    /// Positive control for [`FORBIDDEN_UI_SYMBOLS`]: every wire/transport
    /// needle fires on the encoding or transport reach it forbids, and none of
    /// them touches the pure-data `vnext` view model the crate is allowed to
    /// name.
    #[test]
    fn forbidden_wire_symbol_needles_fire_and_spare_the_view_model() {
        for &(needle, reintroduction) in FORBIDDEN_UI_SYMBOLS {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the wire/transport use it forbids: {reintroduction}"
            );
            for sanctioned in SANCTIONED_VIEW_MODEL_SHAPES {
                assert!(
                    !sym(sanctioned, needle),
                    "needle `{needle}` rejects the sanctioned vnext view model `{sanctioned}`"
                );
            }
        }
    }

    #[test]
    fn status_vocabulary_has_exactly_one_owner() {
        let status_path =
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src/status.rs"));
        let status = std::fs::read_to_string(status_path).expect("read az-editor-ui status.rs");
        for owned in STATUS_MODULE_OWNED_TABLES {
            assert!(
                sym(&status, owned),
                "the shared status module must own `{owned}`"
            );
        }

        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("scan az-editor-ui sources");
        let mut scanned = 0_usize;
        let mut violations = Vec::new();
        for source in &sources {
            if source.path == status_path {
                continue;
            }
            scanned += 1;
            for &(retired, _) in RETIRED_UI_STATUS_TABLES {
                if sym(&source.source, retired) {
                    violations.push(format!("{} names `{retired}`", source.path.display()));
                }
            }
        }

        assert!(scanned > 1, "status guard scanned no sibling ui sources");
        assert!(
            violations.is_empty(),
            "status labels and tones live in az-editor-ui `status.rs` only; callers read them off the enum:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the guard above: each retired name must still match
    /// a table that reintroduces it, and none may match the method calls that
    /// replaced them — otherwise the guard passes by being unable to fail.
    #[test]
    fn retired_status_table_needles_fire_and_spare_the_shared_methods() {
        for &(retired, reintroduction) in RETIRED_UI_STATUS_TABLES {
            assert!(
                sym(reintroduction, retired),
                "needle `{retired}` cannot match the table it forbids: {reintroduction}"
            );
            for call in [
                "status.label()",
                "entry.status.tone().color(theme)",
                "state.state.label().to_owned()",
                "phase.tone()",
            ] {
                assert!(
                    !sym(call, retired),
                    "needle `{retired}` rejects the shared status method call `{call}`"
                );
            }
        }
    }

    /// Symbol-level check over production source text: matches identifier and
    /// punctuation sequences regardless of formatting; comments and string-
    /// literal contents never match. See `az_architecture_guard::symbols_contain`.
    fn sym(source: &str, snippet: &str) -> bool {
        az_architecture_guard::symbols_contain(source, snippet)
    }

    #[test]
    fn settings_module_is_data_only() {
        let settings =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/settings.rs"))
                .expect("read az-editor-ui settings.rs");

        assert!(
            sym(&settings, "pub struct EditorSettings")
                && sym(&settings, "pub struct SettingsStore"),
            "settings module must expose only the editor settings data shape and GPUI global"
        );
        for &(forbidden, _) in SETTINGS_FILE_AND_FORMAT_POLICY {
            assert!(
                !sym(&settings, forbidden),
                "az-editor-ui settings must stay data-only; editor shell owns `{forbidden}`"
            );
        }
    }

    #[test]
    fn ui_crate_does_not_own_host_directory_or_file_watcher_policy() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-editor-ui Cargo.toml");
        let manifest = toml::from_str::<Value>(&manifest).expect("parse az-editor-ui Cargo.toml");
        let deps = manifest["dependencies"]
            .as_table()
            .expect("dependencies table");
        let location = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/theme/location.rs"
        ))
        .expect("read az-editor-ui theme/location.rs");
        let manager =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/theme/manager.rs"))
                .expect("read az-editor-ui theme/manager.rs");

        for forbidden in ["directories", "notify"] {
            assert!(
                !deps.contains_key(forbidden),
                "az-editor-ui must receive explicit shell-resolved paths instead of depending on `{forbidden}`"
            );
        }
        for &(forbidden, _) in THEME_HOST_DIRECTORY_POLICY {
            assert!(
                !sym(&location, forbidden) && !sym(&manager, forbidden),
                "theme path policy belongs to az-editor shell, not az-editor-ui: `{forbidden}`"
            );
        }
    }

    /// Positive control for the two data-only guards above: every needle fires
    /// on the policy it forbids, and none of them matches the data-only shapes
    /// the ui crate keeps.
    #[test]
    fn settings_and_theme_policy_needles_fire_and_spare_the_data_only_shapes() {
        const DATA_ONLY_SHAPES: &[&str] = &[
            "pub struct EditorSettings { pub theme: String, pub font_size: f32 }",
            "pub struct SettingsStore { pub settings: EditorSettings }",
            "pub fn load_toml_themes(theme_locations: &[ThemeLocation], cx: &mut gpui::App)",
            "if let ThemeLocation::User(_) = location { locations.push((location.clone(), 2)); }",
        ];

        for &(needle, reintroduction) in SETTINGS_FILE_AND_FORMAT_POLICY
            .iter()
            .chain(THEME_HOST_DIRECTORY_POLICY)
        {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the host policy it forbids: {reintroduction}"
            );
            for shape in DATA_ONLY_SHAPES {
                assert!(
                    !sym(shape, needle),
                    "needle `{needle}` rejects the data-only shape `{shape}`"
                );
            }
        }
    }

    #[test]
    fn reflected_ui_edits_carry_project_host_commands() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct ApplyReflectedPrefabEdit")
            .expect("reflected Prefab edit action exists");
        let action_end = actions[action_start..]
            .find("/// Invokes an editor action")
            .map(|offset| action_start + offset)
            .expect("reflected Prefab edit action is followed by reflected invocation action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub command: PrefabEditCommand"),
            "reflected edits must carry the native project-host command"
        );
        for &(obsolete, _) in OBSOLETE_AUTHORED_EDIT_ACTIONS {
            assert!(
                !sym(&actions, &format!("pub struct {obsolete}")),
                "obsolete authored edit action remains: `{obsolete}`"
            );
        }
    }

    #[test]
    fn authored_document_creation_actions_are_schema_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct CreateAuthoredDocument")
            .expect("CreateAuthoredDocument action exists");
        let action_end = actions[action_start..]
            .find("/// Refresh `GameData` catalog")
            .map(|offset| action_start + offset)
            .expect("CreateAuthoredDocument action is followed by GameData refresh action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub struct CreateAuthoredDocument")
                && sym(action_body, "pub root_schema: String"),
            "authored document creation must carry the selected root schema"
        );
        for &(forbidden, _) in CATALOG_PLACEMENT_FIELDS {
            assert!(
                !sym(action_body, forbidden),
                "CreateAuthoredDocument must not carry catalog placement field `{forbidden}`; the editor shell resolves placement from the loaded project-host schema catalog"
            );
        }
    }

    #[test]
    fn graph_document_creation_actions_are_catalog_selection_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct CreateGraphDocument")
            .expect("CreateGraphDocument action exists");
        let action_end = actions[action_start..]
            .find("/// Open an existing visual graph document")
            .map(|offset| action_start + offset)
            .expect("CreateGraphDocument action is followed by open action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub struct CreateGraphDocument")
                && sym(action_body, "pub graph_type: String")
                && sym(action_body, "pub document_name: String"),
            "graph document creation must carry the selected graph type and document stem"
        );
        for &(forbidden, _) in CATALOG_PLACEMENT_FIELDS {
            assert!(
                !sym(action_body, forbidden),
                "CreateGraphDocument must not carry catalog placement field `{forbidden}`; editor-core derives placement from the project-host graph catalog"
            );
        }
    }

    #[test]
    fn graph_node_add_actions_are_descriptor_selection_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct AddGraphNode")
            .expect("AddGraphNode action exists");
        let action_end = actions[action_start..]
            .find("/// Move a node")
            .map(|offset| action_start + offset)
            .expect("AddGraphNode action is followed by move action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub struct AddGraphNode")
                && sym(action_body, "pub node_type: String")
                && sym(action_body, "pub node_type_version: u32"),
            "graph node add actions must carry the selected descriptor identity"
        );
        for &(forbidden, _) in ADD_GRAPH_NODE_MODEL_DETAIL {
            assert!(
                !sym(action_body, forbidden),
                "AddGraphNode must not carry model construction detail `{forbidden}`; editor-core derives it from the project-host node catalog"
            );
        }
    }

    #[test]
    fn graph_node_move_actions_are_node_identity_and_layout_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct MoveGraphNode")
            .expect("MoveGraphNode action exists");
        let action_end = actions[action_start..]
            .find("/// Connect two ports")
            .map(|offset| action_start + offset)
            .expect("MoveGraphNode action is followed by connect action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub node_id: String")
                && sym(action_body, "pub x: f32")
                && sym(action_body, "pub y: f32")
                && sym(action_body, "pub struct RemoveGraphNode"),
            "graph node move/remove actions must carry only node identity and target layout coordinates"
        );
        for &(forbidden, _) in MOVE_GRAPH_NODE_MODEL_DETAIL {
            assert!(
                !sym(action_body, forbidden),
                "graph node actions must not carry graph model detail `{forbidden}`; editor-core converts them into validated graph commands"
            );
        }
    }

    #[test]
    fn graph_route_anchor_move_actions_are_anchor_identity_and_point_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct MoveGraphRouteAnchor")
            .expect("MoveGraphRouteAnchor action exists");
        let action_end = actions[action_start..]
            .find("/// Connect two ports")
            .map(|offset| action_start + offset)
            .expect("MoveGraphRouteAnchor action is followed by connect action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub connection_id: String")
                && sym(action_body, "pub anchor_id: String")
                && sym(action_body, "pub x: f32")
                && sym(action_body, "pub y: f32"),
            "graph route-anchor move actions must carry only route-anchor identity and target coordinates"
        );
        for &(forbidden, _) in MOVE_GRAPH_ROUTE_ANCHOR_MODEL_DETAIL {
            assert!(
                !sym(action_body, forbidden),
                "MoveGraphRouteAnchor must not carry graph route detail `{forbidden}`; editor-core preserves it from the validated snapshot"
            );
        }
    }

    #[test]
    fn graph_comment_actions_are_text_and_bounds_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct CreateGraphComment")
            .expect("CreateGraphComment action exists");
        let action_end = actions[action_start..]
            .find("/// Connect two ports")
            .map(|offset| action_start + offset)
            .expect("graph comment actions are followed by connect action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub struct CreateGraphComment")
                && sym(action_body, "pub text: String")
                && sym(action_body, "pub x: f32")
                && sym(action_body, "pub y: f32")
                && sym(action_body, "pub width: f32")
                && sym(action_body, "pub height: f32")
                && sym(action_body, "pub struct MoveGraphComment")
                && sym(action_body, "pub struct SetGraphCommentText")
                && sym(action_body, "pub struct RemoveGraphComment")
                && sym(action_body, "pub comment_id: String")
                && sym(action_body, "pub text: String"),
            "graph comment actions must carry only UI-normalized comment identity/text/bounds"
        );
        for &(forbidden, _) in GRAPH_COMMENT_MODEL_DETAIL {
            assert!(
                !sym(action_body, forbidden),
                "graph comment actions must not carry graph model detail `{forbidden}`; editor-core converts them into validated commands"
            );
        }
    }

    #[test]
    fn graph_port_connect_actions_are_endpoint_identity_only() {
        let actions =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/actions.rs"))
                .expect("read az-editor-ui actions.rs");
        let action_start = actions
            .find("pub struct ConnectGraphPorts")
            .expect("ConnectGraphPorts action exists");
        let action_end = actions[action_start..]
            .find("pub struct SetReflectedGraphPortValue")
            .map(|offset| action_start + offset)
            .expect("ConnectGraphPorts action is followed by graph input action");
        let action_body = &actions[action_start..action_end];

        assert!(
            sym(action_body, "pub from_node_id: String")
                && sym(action_body, "pub from_port_id: u32")
                && sym(action_body, "pub to_node_id: String")
                && sym(action_body, "pub to_port_id: u32"),
            "graph port connect actions must carry only endpoint identities"
        );
        for &(forbidden, _) in CONNECT_GRAPH_PORTS_MODEL_DETAIL {
            assert!(
                !sym(action_body, forbidden),
                "ConnectGraphPorts must not carry graph model detail `{forbidden}`; editor-core creates the validated graph command"
            );
        }
    }

    #[test]
    fn graph_panel_renders_project_host_creation_catalog() {
        let panel =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/graph.rs"))
                .expect("read az-editor-ui graph panel");

        assert!(
            sym(&panel, "GraphCreationCatalogProjectionData")
                && sym(&panel, "GraphNodePaletteProjectionData")
                && sym(&panel, "GraphCompilerBackendProjectionData")
                && sym(&panel, "GraphRuntimeExecutionStrategyProjectionData")
                && sym(&panel, "graph_type_execution_label")
                && sym(&panel, "render_graph_catalog_sidebar")
                && sym(&panel, "render_graph_type_catalog_row")
                && sym(&panel, "render_graph_node_catalog_row")
                && sym(&panel, "render_node_nudge_controls")
                && sym(&panel, "pending_output_port")
                && sym(&panel, "ConnectGraphPorts")
                && sym(&panel, "render_input_value_controls")
                && sym(&panel, "selected_node_id")
                && sym(&panel, "selected_comment_id")
                && sym(&panel, "render_graph_selection_inspector")
                && sym(&panel, "SetReflectedGraphPortValue")
                && sym(&panel, "RemoveGraphNode")
                && sym(&panel, "CreateGraphComment")
                && sym(&panel, "MoveGraphComment")
                && sym(&panel, "SetGraphCommentText")
                && sym(&panel, "RemoveGraphComment")
                && sym(&panel, "CreateGraphDocument"),
            "graph panel must render project-host graph and node creation capabilities"
        );
        for &(forbidden, _) in GRAPH_PANEL_DOCUMENT_CONSTRUCTION {
            assert!(
                !sym(&panel, forbidden),
                "graph panel must not hardcode graph source placement or construct graph documents itself"
            );
        }
    }

    /// Positive control for every action-payload guard above: each needle
    /// fires on the model detail it forbids, and none of them matches the
    /// UI-normalized action payloads the crate actually ships — otherwise the
    /// guards would forbid the shapes they exist to protect.
    #[test]
    fn action_payload_needles_fire_and_spare_the_shipped_actions() {
        const SHIPPED_ACTION_SHAPES: &[&str] = &[
            "pub struct ApplyReflectedPrefabEdit { pub command: PrefabEditCommand }",
            "pub struct CreateAuthoredDocument { pub root_schema: String }",
            "pub struct CreateGraphDocument { pub graph_type: String, pub document_name: String }",
            "pub struct AddGraphNode { pub node_type: String, pub node_type_version: u32 }",
            "pub struct MoveGraphNode { pub node_id: String, pub x: f32, pub y: f32 }",
            "pub struct MoveGraphRouteAnchor { pub connection_id: String, pub anchor_id: String }",
            "pub struct CreateGraphComment { pub text: String, pub width: f32, pub height: f32 }",
            "pub struct ConnectGraphPorts { pub from_node_id: String, pub from_port_id: u32 }",
            "render_graph_catalog_sidebar(&projection.creation_catalog, cx)",
        ];

        for &(obsolete, reintroduction) in OBSOLETE_AUTHORED_EDIT_ACTIONS {
            let needle = format!("pub struct {obsolete}");
            assert!(
                sym(reintroduction, &needle),
                "needle `{needle}` cannot match the retired action it forbids: {reintroduction}"
            );
            for shipped in SHIPPED_ACTION_SHAPES {
                assert!(
                    !sym(shipped, &needle),
                    "needle `{needle}` rejects the shipped action `{shipped}`"
                );
            }
        }

        for &(needle, reintroduction) in CATALOG_PLACEMENT_FIELDS
            .iter()
            .chain(ADD_GRAPH_NODE_MODEL_DETAIL)
            .chain(MOVE_GRAPH_NODE_MODEL_DETAIL)
            .chain(MOVE_GRAPH_ROUTE_ANCHOR_MODEL_DETAIL)
            .chain(GRAPH_COMMENT_MODEL_DETAIL)
            .chain(CONNECT_GRAPH_PORTS_MODEL_DETAIL)
            .chain(GRAPH_PANEL_DOCUMENT_CONSTRUCTION)
        {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the model detail it forbids: {reintroduction}"
            );
            for shipped in SHIPPED_ACTION_SHAPES {
                assert!(
                    !sym(shipped, needle),
                    "needle `{needle}` rejects the shipped action `{shipped}`"
                );
            }
        }
    }

    #[test]
    fn view_menu_exposes_visual_graph_workspace() {
        let menu = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/menu/mod.rs"))
            .expect("read az-editor-ui menu");

        assert!(
            sym(&menu, "Menu::new(\"View\")")
                && az_architecture_guard::source_literal_contains(&menu, "Visual Graph")
                && sym(&menu, "actions::ToggleGraphPanel"),
            "View menu must expose the visual graph workspace as a dogfood surface"
        );
    }

    #[test]
    fn authored_outliner_uses_editor_owned_creation_capabilities() {
        let outliner = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/panels/outliner.rs"
        ))
        .expect("read az-editor-ui outliner.rs");

        assert!(
            sym(&outliner, "EditorCreatableAuthoredSchemas"),
            "authored outliner must render editor-owned creatable schema capabilities"
        );
        assert!(
            sym(&outliner, "status_error")
                && az_architecture_guard::source_literal_contains(
                    &outliner,
                    "Project-host authored data error:"
                ),
            "authored outliner must surface project-host/controller errors instead of falling back to empty local state"
        );
        for &(forbidden, _) in OUTLINER_PLACEMENT_DETAIL {
            assert!(
                !sym(&outliner, forbidden),
                "authored outliner must not inspect source-document placement detail `{forbidden}`; az-editor derives the UI creation view from the project-host schema catalog"
            );
        }
        assert!(
            !az_architecture_guard::source_literal_contains(
                &outliner,
                OUTLINER_SOURCE_ROOT_HINT_KEY
            ),
            "authored outliner must not read the `{OUTLINER_SOURCE_ROOT_HINT_KEY}` placement hint key; az-editor derives the UI creation view from the project-host schema catalog"
        );
    }

    #[test]
    fn outliner_filtering_stays_allocation_free_per_repaint() {
        let outliner = az_architecture_guard::production_source_without_cfg_test_modules(
            &std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/src/panels/outliner.rs"
            ))
            .expect("read az-editor-ui outliner.rs"),
        );

        assert!(
            sym(&outliner, "fn contains_ignore_ascii_case"),
            "the outliner must fold ASCII case at comparison time instead of lowercasing copies"
        );

        let mut violations = Vec::new();
        for &(needle, reason, _) in OUTLINER_PER_REPAINT_ALLOCATIONS {
            let count = az_architecture_guard::symbols_count(&outliner, needle);
            if count > 0 {
                violations.push(format!("`{needle}` appears {count}x ({reason})"));
            }
        }

        assert!(
            violations.is_empty(),
            "outliner filtering must allocate nothing per keystroke or repaint; it borrows the globals it renders:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the guard above: each needle must still fire on the
    /// allocation it forbids, and none may match the borrowed shape that
    /// replaced it — otherwise the guard passes by being unable to fail.
    #[test]
    fn outliner_allocation_needles_fire_and_spare_the_borrowed_shapes() {
        for &(needle, _, reintroduction) in OUTLINER_PER_REPAINT_ALLOCATIONS {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the allocation it forbids"
            );
            for borrowed in OUTLINER_BORROWED_SHAPES {
                assert!(
                    !sym(borrowed, needle),
                    "needle `{needle}` rejects the allocation-free shape `{borrowed}`"
                );
            }
        }
    }

    #[test]
    fn graph_panel_render_stays_allocation_free_per_repaint() {
        let graph = az_architecture_guard::production_source_without_cfg_test_modules(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/panels/graph.rs"))
                .expect("read az-editor-ui graph.rs"),
        );

        assert!(
            sym(&graph, "static NO_GRAPH_PROJECTION"),
            "the graph panel must borrow a static empty projection instead of building one per repaint"
        );
        assert!(
            sym(&graph, "fn graph_route_anchor_element_id"),
            "route-anchor element ids must come from a dedicated non-allocating constructor"
        );
        assert!(
            !az_architecture_guard::source_literal_contains(&graph, GRAPH_ROUTE_ANCHOR_FORMAT_ID),
            "every connection re-emits every route anchor per repaint; \
             their element ids must not be formatted strings"
        );

        let mut violations = Vec::new();
        for &(needle, reason, _) in GRAPH_PER_REPAINT_ALLOCATIONS {
            let count = az_architecture_guard::symbols_count(&graph, needle);
            if count > 0 {
                violations.push(format!("`{needle}` appears {count}x ({reason})"));
            }
        }

        assert!(
            violations.is_empty(),
            "the graph panel renders at pointer-event frequency; it borrows the projection and theme it displays:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the guard above: each needle must still fire on the
    /// allocation it forbids, and none may match the borrowed shape that
    /// replaced it — otherwise the guard passes by being unable to fail.
    #[test]
    fn graph_allocation_needles_fire_and_spare_the_borrowed_shapes() {
        for &(needle, _, reintroduction) in GRAPH_PER_REPAINT_ALLOCATIONS {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the allocation it forbids"
            );
            for borrowed in GRAPH_BORROWED_SHAPES {
                assert!(
                    !sym(borrowed, needle),
                    "needle `{needle}` rejects the allocation-free shape `{borrowed}`"
                );
            }
        }

        assert!(
            az_architecture_guard::source_literal_contains(
                "                    \"graph-route-anchor-{}-{}\",",
                GRAPH_ROUTE_ANCHOR_FORMAT_ID
            ),
            "the route-anchor literal needle cannot match the formatted id it forbids"
        );
        assert!(
            !az_architecture_guard::source_literal_contains(
                "graph_route_anchor_element_id(&connection.connection_id, &anchor.anchor_id)",
                GRAPH_ROUTE_ANCHOR_FORMAT_ID
            ),
            "the route-anchor literal needle rejects the allocation-free shape that replaced it"
        );
    }

    #[test]
    fn asset_browser_is_not_a_local_filesystem_navigator() {
        let browser = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/panels/asset_browser.rs"
        ))
        .expect("read az-editor-ui asset_browser.rs");

        assert!(
            sym(&browser, "EditorAssetBrowserStatus"),
            "asset browser must render editor-supplied asset-processor status"
        );
        assert!(
            sym(&browser, "AssetSourcePreviewLocator"),
            "asset browser image preview must use a typed source preview locator instead of raw absolute path strings"
        );
        assert!(
            sym(&browser, "status_error")
                && az_architecture_guard::source_literal_contains(
                    &browser,
                    "Asset processor error:"
                ),
            "asset browser must surface asset-processor status errors instead of falling back to local filesystem state"
        );
        for &(forbidden, _) in ASSET_BROWSER_FILESYSTEM_NAVIGATION {
            assert!(
                !sym(&browser, forbidden),
                "asset browser must not carry local filesystem navigation state `{forbidden}`; asset-processor workspace views own source roots and asset entries"
            );
        }
        for &(label, _) in ASSET_BROWSER_FILESYSTEM_LABELS {
            assert!(
                !az_architecture_guard::source_literal_contains(&browser, label),
                "asset browser must not present itself as a local file browser (`{label}`); asset-processor workspace views own source roots and asset entries"
            );
        }
    }

    /// Positive control for the outliner and asset-browser guards above: each
    /// needle fires on the placement or filesystem-navigation shape it forbids,
    /// and none of them matches the editor-owned projections the panels render.
    ///
    /// The two label needles and the placement hint key are checked as contract
    /// text: they only ever appear inside string literals, whose contents [`sym`]
    /// strips, so as symbol needles they could never fire (ticket 042).
    #[test]
    fn panel_placement_needles_fire_and_spare_the_editor_owned_projections() {
        const EDITOR_OWNED_PROJECTIONS: &[&str] = &[
            "let schemas = cx.try_global::<EditorCreatableAuthoredSchemas>();",
            "let status = cx.try_global::<EditorAssetBrowserStatus>();",
            "let locator = AssetSourcePreviewLocator::new(entry.asset_id);",
        ];

        for &(needle, reintroduction) in OUTLINER_PLACEMENT_DETAIL
            .iter()
            .chain(ASSET_BROWSER_FILESYSTEM_NAVIGATION)
        {
            assert!(
                sym(reintroduction, needle),
                "needle `{needle}` cannot match the placement detail it forbids: {reintroduction}"
            );
            for projection in EDITOR_OWNED_PROJECTIONS {
                assert!(
                    !sym(projection, needle),
                    "needle `{needle}` rejects the editor-owned projection `{projection}`"
                );
            }
        }

        for &(label, reintroduction) in ASSET_BROWSER_FILESYSTEM_LABELS {
            assert!(
                az_architecture_guard::source_literal_contains(reintroduction, label),
                "label needle `{label}` cannot match the local-navigator wording it forbids: {reintroduction}"
            );
            for projection in EDITOR_OWNED_PROJECTIONS {
                assert!(
                    !az_architecture_guard::source_literal_contains(projection, label),
                    "label needle `{label}` rejects the editor-owned projection `{projection}`"
                );
            }
        }
        assert!(
            az_architecture_guard::source_literal_contains(
                "if hint.key == \"project:source-root\" { root = hint.value.clone(); }",
                OUTLINER_SOURCE_ROOT_HINT_KEY
            ),
            "the placement hint key cannot match the hint lookup it forbids"
        );
        assert!(
            !az_architecture_guard::source_literal_contains(
                "let schemas = cx.try_global::<EditorCreatableAuthoredSchemas>();",
                OUTLINER_SOURCE_ROOT_HINT_KEY
            ),
            "the placement hint key rejects the editor-owned creation projection"
        );
    }

    fn forbidden_ui_dependencies(manifest: &Value, workspace_manifest: &Value) -> Vec<String> {
        let mut exact_names = Vec::new();
        exact_names.extend_from_slice(FORBIDDEN_UI_DEPS);
        exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
        forbidden_production_dependencies(
            manifest,
            workspace_manifest,
            DependencyBoundary::new(
                &exact_names,
                PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
                PROJECT_OR_FORMAT_PATH_SEGMENTS,
            ),
        )
    }
}
