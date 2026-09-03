use az_architecture_guard::{
    COMPATIBILITY_FORMAT_DEPENDENCIES, DependencyBoundary, GEM_INFRASTRUCTURE_DEPENDENCIES,
    PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS, PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
    forbidden_production_dependencies,
};
use toml::Value;

// `az-asset` is a permitted production dependency: the processor owns asset
// catalog publication (`publish_asset_catalog`) and writes the catalog file
// through `az_asset`'s format types, mirroring Lumberyard's AssetProcessor
// owning assetcatalog emission.
const FORBIDDEN_PRODUCTION_DEPENDENCIES: &[&str] = &[
    "az-daemon",
    "az-editor",
    "az-editor-derive",
    "az-editor-inspector",
    "az-editor-ui",
    "az-engine",
    "az-framework",
    "az-project-host",
    "az-proto-daemon",
    "az-proto-project",
    "az-proto-runtime",
    "az-runtime-host",
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
fn production_asset_processor_dependencies_stay_generic_asset_queue_adapter() {
    let manifest: Value = toml::from_str(include_str!("../Cargo.toml")).unwrap();
    let workspace_manifest: Value = toml::from_str(include_str!("../../../../Cargo.toml")).unwrap();
    let violations = forbidden_asset_processor_dependencies(&manifest, &workspace_manifest);

    assert!(
        violations.is_empty(),
        "az-asset-processor production dependencies must stay a generic asset-queue service adapter; forbidden direct deps: {}",
        violations.join(", ")
    );
}

#[test]
fn asset_processor_dependency_guard_rejects_aliases_targets_paths_and_workspace_packages() {
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        runtime = { package = "az-runtime-host", path = "../az-runtime-host" }
        authored = { package = "az-schema", path = "../az-schema" }

        [target.'cfg(windows)'.dependencies]
        engine = { package = "az-engine", path = "../az-engine" }
        gem = { path = "../gems/lmbr-central" }

        [target.'cfg(unix)'.build-dependencies]
        project = { workspace = true }
        "#,
    )
    .unwrap();
    let workspace_manifest: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        project = { package = "sample-plugin", path = "../projects/sample-plugin" }
        "#,
    )
    .unwrap();

    assert_eq!(
        forbidden_asset_processor_dependencies(&manifest, &workspace_manifest),
        vec![
            "dependencies.authored(package az-schema)",
            "dependencies.runtime(package az-runtime-host)",
            "target.cfg(unix).build-dependencies.project(workspace package sample-plugin)",
            "target.cfg(windows).dependencies.engine(package az-engine)",
            "target.cfg(windows).dependencies.gem(path ../gems/lmbr-central)",
        ]
    );
}

#[test]
fn asset_worker_source_bytes_stay_db_payload_only() {
    let source = include_str!("../../asset-worker/src/lib.rs");

    assert!(
        source.contains("MissingSourcePayload"),
        "workers must explicitly fail malformed leases without DB-authored payload side channels"
    );
    for forbidden in [
        "std::fs::read(&source_path)",
        "Path::new(&attempt.source_root).join(&attempt.source_path)",
    ] {
        assert!(
            !source.contains(forbidden),
            "asset workers must not use `{forbidden}` as a source-file fallback authority"
        );
    }
}

#[test]
fn asset_worker_physical_boundary_stays_db_free() {
    let worker_manifest: toml::Value =
        toml::from_str(include_str!("../../asset-worker/Cargo.toml"))
            .expect("asset-worker manifest is valid TOML");
    let workspace_manifest: toml::Value = toml::from_str(include_str!("../../../../Cargo.toml"))
        .expect("workspace manifest is valid TOML");
    let dependency_names = worker_dependency_names(&worker_manifest, &workspace_manifest);
    for forbidden in ["az-assetdb", "az-asset-processor"] {
        assert!(
            !dependency_names.iter().any(|name| name == forbidden),
            "asset-worker must not depend on `{forbidden}` in any Cargo dependency table; found {dependency_names:?}"
        );
    }

    for (path, source) in [
        ("src/lib.rs", include_str!("../../asset-worker/src/lib.rs")),
        (
            "src/transport.rs",
            include_str!("../../asset-worker/src/transport.rs"),
        ),
    ] {
        assert!(
            !source.contains("az_assetdb") && !source.contains("az-assetdb"),
            "asset-worker {path} must not reach the asset database crate"
        );
        assert!(
            !source.contains("az_asset_processor") && !source.contains("az-asset-processor"),
            "asset-worker {path} must not reach the asset-processor crate"
        );
    }
}

fn worker_dependency_names(
    manifest: &toml::Value,
    workspace_manifest: &toml::Value,
) -> Vec<String> {
    let mut names = Vec::new();
    collect_worker_dependency_names(manifest, workspace_manifest, &mut names);
    names
}

fn collect_worker_dependency_names(
    value: &toml::Value,
    workspace_manifest: &toml::Value,
    names: &mut Vec<String>,
) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, value) in table {
        if matches!(
            key.as_str(),
            "dependencies" | "dev-dependencies" | "build-dependencies"
        ) {
            let Some(dependencies) = value.as_table() else {
                continue;
            };
            for (alias, dependency) in dependencies {
                names.push(alias.clone());
                let package = dependency.as_table().and_then(|dependency| {
                    dependency
                        .get("package")
                        .and_then(toml::Value::as_str)
                        .or_else(|| {
                            dependency
                                .get("workspace")
                                .and_then(toml::Value::as_bool)
                                .filter(|workspace| *workspace)
                                .and_then(|_| {
                                    workspace_manifest
                                        .get("workspace")
                                        .and_then(toml::Value::as_table)
                                        .and_then(|workspace| workspace.get("dependencies"))
                                        .and_then(toml::Value::as_table)
                                        .and_then(|dependencies| dependencies.get(alias))
                                        .and_then(toml::Value::as_table)
                                        .and_then(|dependency| dependency.get("package"))
                                        .and_then(toml::Value::as_str)
                                })
                        })
                });
                if let Some(package) = package {
                    names.push(package.to_string());
                }
            }
        } else {
            collect_worker_dependency_names(value, workspace_manifest, names);
        }
    }
}

fn forbidden_asset_processor_dependencies(
    manifest: &Value,
    workspace_manifest: &Value,
) -> Vec<String> {
    let mut exact_names = FORBIDDEN_PRODUCTION_DEPENDENCIES.to_vec();
    exact_names.extend_from_slice(COMPATIBILITY_FORMAT_DEPENDENCIES);
    forbidden_production_dependencies(
        manifest,
        workspace_manifest,
        // `az-gem-contract` is the composition contract, not a gem: the
        // processor takes registry entry types and `Registries` from it and
        // nothing else, which is what keeps it a generic asset-queue adapter
        // instead of linking project code.
        DependencyBoundary::new(
            &exact_names,
            PROJECT_OR_GAME_DEPENDENCY_PREFIXES,
            PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS,
        )
        .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES),
    )
}
