//! Inspector system for the `AZoth` Editor.
//!
//! Provides the ADR-0022 reflected-inspection projection and edit bindings.

pub mod reflected;

pub use reflected::{
    AddComponentCapabilities, AddComponentEvaluation, AddComponentEvaluationState,
    ReflectedAddComponent, ReflectedComponentInspection, ReflectedCurrentValue,
    ReflectedDefaultAvailability, ReflectedDefaultValue, ReflectedEditBinding,
    ReflectedEntityInspection, ReflectedInspectionChild, ReflectedInspectionField,
    ReflectedInspectionInput, ReflectedInspectionModel, ReflectedListItem, ReflectedMapEntry,
    ReflectedMapEntryBinding, ReflectedMapValueEntry, ReflectedOverrideOperation,
    ReflectedPrefabSelection, ReflectedProjectionError, ReflectedScalar, ReflectedValidationState,
    ReflectedValue, ReflectedValueNode, ReflectedVariantSelection, WidgetFamily, WidgetSpec,
    decode_reflected_envelope, decode_standalone_reflected_value,
    standalone_reflected_widget_family,
};

#[cfg(test)]
mod architecture_tests {
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, PROJECT_OR_FORMAT_PATH_SEGMENTS,
        PROJECT_OR_GAME_DEPENDENCY_PREFIXES, collect_production_rust_sources,
        forbidden_production_dependencies,
    };

    use toml::Value;

    const FORBIDDEN_INSPECTOR_DEPS: &[&str] = &[
        "az-asset",
        "az-asset-builder",
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-ui",
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
        // (`vnext` — no Cap'n Proto bindings), which this crate binds and edits
        // as its schema model. Relocating that module was attempted (014) and
        // is blocked by the wire encodings' inherent impls. The boundary this
        // guard protects is *transport and service hosting*, so the wire half
        // stays forbidden via the source needles in
        // `inspector_source_stays_schema_binding_only`.
        "az-proto-runtime",
        "az-proto-session",
        "az-rpc",
        "az-runtime-host",
        "az-session",
        "az-sessiond",
        "bevy",
        "gpui",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "sample-plugin",
    ];
    #[test]
    fn inspector_crate_stays_schema_binding_only() {
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .expect("read az-editor-inspector Cargo.toml");
        let manifest =
            toml::from_str::<Value>(&manifest).expect("parse az-editor-inspector Cargo.toml");
        let workspace_manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"))
                .expect("read workspace Cargo.toml");
        let workspace_manifest =
            toml::from_str::<Value>(&workspace_manifest).expect("parse workspace Cargo.toml");
        let violations = forbidden_inspector_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-editor-inspector must stay schema/edit binding only; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn inspector_crate_rejects_forbidden_workspace_package_aliases() {
        let manifest = toml::from_str::<Value>(
            r"
            [dependencies]
            project-schema = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>(
            r#"
            [workspace.dependencies]
            project-schema = { package = "sample-plugin", version = "0.1" }
            "#,
        )
        .unwrap();
        let violations = forbidden_inspector_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec!["dependencies.project-schema(workspace package sample-plugin)"]
        );
    }

    #[test]
    fn inspector_crate_rejects_legacy_format_and_gem_package_aliases() {
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
        let violations = forbidden_inspector_dependencies(&manifest, &workspace_manifest);

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
    fn inspector_crate_rejects_forbidden_path_only_dependencies() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            db = { path = "../crates/az/assetdb" }
            terrain = { path = "../gems/legacy-terrain" }
            "#,
        )
        .unwrap();
        let workspace_manifest = toml::from_str::<Value>("[workspace.dependencies]").unwrap();
        let violations = forbidden_inspector_dependencies(&manifest, &workspace_manifest);

        assert_eq!(
            violations,
            vec![
                "dependencies.db(path ../crates/az/assetdb)",
                "dependencies.terrain(path ../gems/legacy-terrain)",
            ]
        );
    }

    /// Service, protocol, database, process, view-layer, and webview boundaries
    /// the inspector must not open. Third column is the reintroduction each
    /// needle must still fire on, and the control below iterates this same
    /// table.
    ///
    /// The matcher here is raw substring containment, not
    /// `az_architecture_guard::symbols_contain`, so these needles also fire
    /// inside comments and string literals. That makes every non-empty needle
    /// trivially matchable by a sample built out of the needle itself — the
    /// reintroductions are therefore written out as real code, which is what
    /// makes the control able to fail (ticket 042).
    const FORBIDDEN_INSPECTOR_SOURCE_BOUNDARIES: &[(&str, &str, &str)] = &[
        ("use gpui", "GPUI import", "use gpui::{App, Context};"),
        (
            "gpui::",
            "GPUI API",
            "fn render(&self, cx: &mut gpui::App) -> gpui::AnyElement {",
        ),
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
        (
            "vnext_wire",
            "vNext wire encoding module",
            "use az_proto_project::vnext_wire::encode_reflected_value;",
        ),
        (
            "project_capnp",
            "generated Cap'n Proto bindings",
            "use az_proto_project::project_capnp::project_host;",
        ),
        ("capnp_rpc", "Cap'n Proto RPC", "use capnp_rpc::RpcSystem;"),
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

    /// The sanctioned schema-binding shapes: the pure-data `vnext` contract
    /// this crate binds and edits. No needle above may match one of these.
    const SANCTIONED_SCHEMA_BINDING_SHAPES: &[&str] = &[
        "use az_proto_project::vnext::{PrefabEditCommand, ReflectedValue};",
        "let kind = az_proto_project::vnext::ReflectedTypeKind::Struct;",
        "pub struct ReflectedEditBinding { pub path: String }",
    ];

    #[test]
    fn inspector_source_stays_schema_binding_only() {
        let sources = collect_production_rust_sources(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
            .expect("collect az-editor-inspector production sources");
        let mut violations = Vec::new();

        for source in sources {
            for &(needle, reason, _) in FORBIDDEN_INSPECTOR_SOURCE_BOUNDARIES {
                if source.source.contains(needle) {
                    violations.push(format!(
                        "{} contains `{needle}` ({reason})",
                        source.path.display()
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "az-editor-inspector must stay a schema/edit binding layer; forbidden production source boundary use:\n{}",
            violations.join("\n")
        );
    }

    /// Positive control for the guard above: every needle fires on the boundary
    /// it forbids, and none of them matches the sanctioned `vnext` schema
    /// binding the crate is built on.
    #[test]
    fn forbidden_inspector_boundary_needles_fire_and_spare_the_schema_binding() {
        for &(needle, _, reintroduction) in FORBIDDEN_INSPECTOR_SOURCE_BOUNDARIES {
            assert!(
                reintroduction.contains(needle),
                "needle `{needle}` cannot match the boundary it forbids: {reintroduction}"
            );
            for sanctioned in SANCTIONED_SCHEMA_BINDING_SHAPES {
                assert!(
                    !sanctioned.contains(needle),
                    "needle `{needle}` rejects the sanctioned schema binding `{sanctioned}`"
                );
            }
        }
    }

    fn forbidden_inspector_dependencies(
        manifest: &Value,
        workspace_manifest: &Value,
    ) -> Vec<String> {
        let mut exact_names = Vec::new();
        exact_names.extend_from_slice(FORBIDDEN_INSPECTOR_DEPS);
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
