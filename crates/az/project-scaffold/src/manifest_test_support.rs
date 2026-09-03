use az_architecture_guard::{DependencyBoundary, DependencyScan, forbidden_manifest_dependencies};
use toml::Value;

/// Packages an Azoth project's own Cargo manifests must never depend on.
///
/// The scaffold no longer strips these from a project's manifests: the
/// `crates/game` cluster it used to rewrite is gone, and every manifest a
/// project owns is now either engine-generated or gem-authored. What survives
/// is the boundary itself, asserted against everything the scaffold emits.
pub const FORBIDDEN_PROJECT_MANIFEST_DEPENDENCIES: &[&str] = &[
    "az-editor",
    "az-editor-ui",
    "az-editor-inspector",
    "az-engine",
    "az-framework",
    "az-daemon",
    "az-session",
    "az-sessiond",
];
pub const FORBIDDEN_PROJECT_MANIFEST_DEPENDENCY_PREFIXES: &[&str] = &[];

pub fn assert_no_forbidden_manifest_dependencies(
    scope: &str,
    manifest_text: &str,
    exact_names: &[&str],
    name_prefixes: &[&str],
) {
    let manifest = toml::from_str::<Value>(manifest_text)
        .unwrap_or_else(|source| panic!("parse {scope}: {source}"));
    let violations = forbidden_manifest_dependencies(
        &manifest,
        &manifest,
        DependencyBoundary::new(exact_names, name_prefixes, &[]),
        DependencyScan::PROJECT_WORKFLOW_MANIFEST,
    );

    assert!(
        violations.is_empty(),
        "{scope} must not depend on forbidden packages: {}",
        violations.join(", ")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXACT: &[&str] = &["az-editor", "az-engine", "lyshine"];
    const PREFIXES: &[&str] = &["cry-", "project-"];

    #[test]
    fn finds_forbidden_dependency_aliases_workspace_aliases_targets_and_paths() {
        let manifest = toml::from_str::<Value>(
            r#"
            [dependencies]
            editor = { package = "az-editor", path = "../crates/editor/core" }
            harmless = "1"
            engine-path = { path = "../crates/az/engine" }
            legacy-path = { path = "../formats/legacy/cry-chunk-assets" }

            [workspace.dependencies]
            net = { package = "project-game", path = "../formats/legacy/project-game" }

            [target.'cfg(windows)'.dependencies]
            net = { workspace = true }
            ui = { package = "lyshine", path = "../gems/lyshine" }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_manifest_dependencies(
                &manifest,
                &manifest,
                DependencyBoundary::new(EXACT, PREFIXES, &[]),
                DependencyScan::PROJECT_WORKFLOW_MANIFEST,
            ),
            vec![
                "dependencies.editor(package az-editor)",
                "dependencies.engine-path(path ../crates/az/engine)",
                "dependencies.legacy-path(path ../formats/legacy/cry-chunk-assets)",
                "target.cfg(windows).dependencies.net(workspace package project-game)",
                "target.cfg(windows).dependencies.ui(package lyshine)",
                "workspace.dependencies.net(package project-game)",
            ]
        );
    }
}
