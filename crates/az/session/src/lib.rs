//! Editor session supervision.
//!
//! This crate is the UI-free core wrapped by `azd`, `az-sessiond`, and CLI
//! commands.
//! Sessions are durable supervision scopes over an existing workspace. They
//! own current service process records, logs, and recovery metadata; they
//! never create, mutate, or delete the workspace itself.

pub mod lease;
pub mod manager;
pub mod manifest;
pub mod package_roots;
pub mod rpc;
mod status_broker;
pub mod supervisor;
pub mod transport;

pub use lease::*;
pub use manager::*;
pub use manifest::*;
pub use package_roots::*;
pub use rpc::*;
pub use status_broker::SessionStatusPublisher;
pub use supervisor::*;
pub use transport::*;

#[cfg(test)]
mod architecture_tests {
    use az_architecture_guard::{
        COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, PROJECT_OR_FORMAT_PATH_SEGMENTS,
        PROJECT_OR_GAME_DEPENDENCY_PREFIXES, forbidden_production_dependencies,
    };
    use toml::Value;

    const FORBIDDEN_PRODUCTION_DEPENDENCIES: &[&str] = &[
        "az-asset-processor",
        "az-assetdb",
        "az-daemon",
        "az-editor",
        "az-editor-inspector",
        "az-editor-ui",
        "az-engine",
        "az-framework",
        "az-project-host",
        "az-runtime-host",
        "az-sessiond",
        "bevy",
        "gridmate",
        "lmbr-central",
        "lyshine",
        "sample-plugin",
    ];
    #[test]
    fn production_session_core_dependencies_stay_ui_free_and_service_host_free() {
        let manifest: Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
        let workspace_manifest: Value =
            toml::from_str(include_str!("../../../../Cargo.toml")).unwrap();
        let violations = forbidden_session_dependencies(&manifest, &workspace_manifest);

        assert!(
            violations.is_empty(),
            "az-session production dependencies must stay UI-free and service-host-free; forbidden direct deps: {}",
            violations.join(", ")
        );
    }

    #[test]
    fn production_session_core_rejects_forbidden_workspace_aliases() {
        let manifest: Value = toml::from_str(
            r"
            [dependencies]
            project-api = { workspace = true }
            db-api = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str(
            r#"
            [workspace.dependencies]
            project-api = { package = "sample-plugin", path = "projects/sample/crates/plugin" }
            db-api = { package = "az-assetdb", path = "crates/az/assetdb" }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_session_dependencies(&manifest, &workspace_manifest),
            vec![
                "dependencies.db-api(workspace package az-assetdb)",
                "dependencies.project-api(workspace package sample-plugin)",
            ]
        );
    }

    #[test]
    fn production_session_core_rejects_forbidden_target_paths_and_path_only_aliases() {
        let manifest: Value = toml::from_str(
            r#"
            [dependencies]
            terrain = { path = "../gems/legacy-terrain" }

            [target.'cfg(windows)'.dependencies]
            legacy-format = { path = "../formats/legacy/cry-chunk-assets" }
            net = { package = "gridmate", path = "../gridmate" }
            "#,
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str("[workspace.dependencies]").unwrap();

        assert_eq!(
            forbidden_session_dependencies(&manifest, &workspace_manifest),
            vec![
                "dependencies.terrain(path ../gems/legacy-terrain)",
                "target.cfg(windows).dependencies.legacy-format(path ../formats/legacy/cry-chunk-assets)",
                "target.cfg(windows).dependencies.net(package gridmate)",
            ]
        );
    }

    fn forbidden_session_dependencies(manifest: &Value, workspace_manifest: &Value) -> Vec<String> {
        let mut exact_names = FORBIDDEN_PRODUCTION_DEPENDENCIES.to_vec();
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
