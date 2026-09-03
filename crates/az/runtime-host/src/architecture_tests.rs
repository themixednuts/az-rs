use az_architecture_guard::{
    COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, GEM_INFRASTRUCTURE_DEPENDENCIES,
    PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS, PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
    forbidden_production_dependencies,
};
use toml::Value;

const FORBIDDEN_PRODUCTION_DEPENDENCIES: &[&str] = &[
    "az-asset",
    "az-asset-builder",
    "az-asset-processor",
    "az-assetdb",
    "az-daemon",
    "az-editor",
    "az-editor-derive",
    "az-editor-inspector",
    "az-editor-ui",
    "az-engine",
    "az-framework",
    "az-project-host",
    "az-proto-asset",
    "az-proto-daemon",
    "az-proto-session",
    "az-schema",
    "az-session",
    "az-sessiond",
    "bevy",
    "gridmate",
    "lmbr-central",
    "lyshine",
    "sample-plugin",
];
#[test]
fn production_runtime_host_dependencies_stay_engine_owned_projection_adapter() {
    let manifest: Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let workspace_manifest: Value = toml::from_str(include_str!("../../../../Cargo.toml")).unwrap();
    let violations = forbidden_runtime_host_dependencies(&manifest, &workspace_manifest);

    assert!(
        violations.is_empty(),
        "az-runtime-host production dependencies must stay an engine-owned projection adapter; forbidden direct deps: {}",
        violations.join(", ")
    );
}

#[test]
fn runtime_host_dependency_guard_rejects_aliases_targets_paths_and_workspace_packages() {
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        engine = { package = "az-engine", path = "../az-engine" }
        project-host = { package = "az-project-host", path = "../az-project-host" }

        [target.'cfg(windows)'.dependencies]
        session-proto = { package = "az-proto-session", path = "../az-proto-session" }
        client = { path = "../../projects/sample/examples/client" }

        [target.'cfg(unix)'.build-dependencies]
        grid = { workspace = true }
        "#,
    )
    .unwrap();
    let workspace_manifest: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        grid = { package = "gridmate", path = "../gridmate" }
        "#,
    )
    .unwrap();

    assert_eq!(
        forbidden_runtime_host_dependencies(&manifest, &workspace_manifest),
        vec![
            "dependencies.engine(package az-engine)",
            "dependencies.project-host(package az-project-host)",
            "target.cfg(unix).build-dependencies.grid(workspace package gridmate)",
            "target.cfg(windows).dependencies.client(path ../../projects/sample/examples/client)",
            "target.cfg(windows).dependencies.session-proto(package az-proto-session)",
        ]
    );
}

fn forbidden_runtime_host_dependencies(
    manifest: &Value,
    workspace_manifest: &Value,
) -> Vec<String> {
    let mut exact_names = FORBIDDEN_PRODUCTION_DEPENDENCIES.to_vec();
    exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
    forbidden_production_dependencies(
        manifest,
        workspace_manifest,
        DependencyBoundary::new(
            &exact_names,
            PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
            PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS,
        )
        .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES),
    )
}
