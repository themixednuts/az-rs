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
    "az-daemon",
    "az-editor",
    "az-editor-derive",
    "az-editor-inspector",
    "az-editor-ui",
    "az-engine",
    "az-framework",
    "az-proto-asset",
    "az-proto-daemon",
    "az-runtime-host",
    "az-session",
    "az-sessiond",
    "bevy",
    "gridmate",
    "lmbr-central",
    "lyshine",
    "sample-plugin",
];

// Project Host is the process boundary for reflected project sources and the
// GameData catalog. Keep the exceptions at the resolved violation level so
// aliases, other asset-pipeline crates, and every other gem path remain
// forbidden.
const GENERIC_AUTHORING_DEPENDENCY_EXCEPTIONS: &[&str] = &[
    "dependencies.az-asset-builder",
    // ProjectHost speaks the generic source-authoring protocol to Asset
    // Processor; it does not link the processor implementation or builders.
    "dependencies.az-proto-asset",
    "dependencies.gamedata(workspace path gems/gamedata)",
    // Ruled 2026-08-20 (registration cutover): the host applies the engine's
    // reflected floor types directly when composing its type registry — the
    // engine_prefab_type_registry precedent — so the framework dependency is
    // the floor, not a gem leak.
    "dependencies.az-framework",
];

#[test]
fn production_project_host_dependencies_stay_generic_project_adapter() {
    let manifest: Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let workspace_manifest: Value = toml::from_str(include_str!("../../../../Cargo.toml")).unwrap();
    let violations = forbidden_project_host_dependencies(&manifest, &workspace_manifest)
        .into_iter()
        .filter(|violation| !GENERIC_AUTHORING_DEPENDENCY_EXCEPTIONS.contains(&violation.as_str()))
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "az-project-host production dependencies must stay a generic project service adapter; forbidden direct deps: {}",
        violations.join(", ")
    );
}

#[test]
fn project_host_dependency_guard_rejects_aliases_targets_paths_and_workspace_packages() {
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        runtime = { package = "az-runtime-host", path = "../az-runtime-host" }
        legacy = { path = "../formats/legacy/cry-chunk-assets" }

        [target.'cfg(windows)'.dependencies]
        editor = { workspace = true }
        gem = { path = "../gems/lyshine" }

        [target.'cfg(unix)'.build-dependencies]
        asset-proto = { package = "az-proto-asset", path = "../az-proto-asset" }
        "#,
    )
    .unwrap();
    let workspace_manifest: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        editor = { package = "az-editor-ui", path = "../az-editor-ui" }
        "#,
    )
    .unwrap();

    assert_eq!(
        forbidden_project_host_dependencies(&manifest, &workspace_manifest),
        vec![
            "dependencies.legacy(path ../formats/legacy/cry-chunk-assets)",
            "dependencies.runtime(package az-runtime-host)",
            "target.cfg(unix).build-dependencies.asset-proto(package az-proto-asset)",
            "target.cfg(windows).dependencies.editor(workspace package az-editor-ui)",
            "target.cfg(windows).dependencies.gem(path ../gems/lyshine)",
        ]
    );
}

fn forbidden_project_host_dependencies(
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
        // Gem-linking infrastructure is engine infrastructure (ADR 0015), not
        // a gem: project-host publishes process/gem inventory through it.
        .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES),
    )
}
