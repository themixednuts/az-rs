//! Shared architecture-boundary checks for Azoth crates and generated project
//! manifests.
//!
//! It keeps editor, engine, service-host, project, gem, and compatibility-format
//! dependency guards consistent across crate architecture tests and project
//! workflow validation.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use toml::Value;

mod deletion_targets;
mod schema_lock;
mod symbols;

pub use deletion_targets::*;
pub use schema_lock::*;
pub use symbols::*;

/// Dependency name prefixes that identify project, gem, game, or imported
/// legacy-domain crates.
///
/// Production engine/editor/service boundaries compose
/// this instead of spelling their own overlapping prefix lists.
pub const PROJECT_OR_GAME_DEPENDENCY_PREFIXES: &[&str] = &["az-gem-", "cry-"];

/// Path segments that identify project/gem or compatibility-format source
/// trees in this workspace.
pub const PROJECT_OR_FORMAT_PATH_SEGMENTS: &[&str] = &["formats", "gems"];

/// Path segments that identify project/gem, examples, or compatibility-format
/// source trees. Service-host crates use this stricter variant because examples
/// are also outside their production boundary.
pub const PROJECT_FORMAT_OR_EXAMPLE_PATH_SEGMENTS: &[&str] = &["examples", "formats", "gems"];

/// Path segments that also reject workspace-local project packages. The root
/// CLI crate uses this because it is the public engine/editor command surface,
/// not a project implementation crate.
pub const PROJECT_FORMAT_EXAMPLE_OR_WORKSPACE_PROJECT_PATH_SEGMENTS: &[&str] =
    &["examples", "formats", "gems", "projects"];

/// Common compatibility-format crates that should not leak into universal
/// editor, schema, engine, or service boundary crates unless a crate explicitly
/// owns that compatibility layer.
pub const COMPATIBILITY_FORMAT_DEPENDENCIES: &[&str] = &[
    "az-framework-input",
    "az-framework-objectstream",
    "az-lua-bytecode",
    "az-objectstream",
    "az-pak",
    "az-physics-assets",
    "coat-tractmap",
    "lmbr-central-assets",
    "lyshine-canvas",
    "nv-cloth-assets",
    "texture-atlas",
    "texture-atlas-assets",
];

/// Engine-workspace infrastructure crates whose names collide with the
/// `az-gem-` gem-crate prefix.
///
/// The gem *contract* (ADR 0032) is engine infrastructure, not a gem:
/// boundaries that forbid gem crates by prefix may still depend on it
/// explicitly via [`DependencyBoundary::with_exempt_names`].
pub const GEM_INFRASTRUCTURE_DEPENDENCIES: &[&str] = &["az-gem-contract", "az-gem-loader"];

/// Manifest dependency policy for a production-facing crate boundary.
#[derive(Debug, Clone, Copy)]
pub struct DependencyBoundary<'a> {
    /// Exact crate or package names that must not cross the boundary.
    pub exact_names: &'a [&'a str],
    /// Name prefixes that must not cross the boundary.
    pub name_prefixes: &'a [&'a str],
    /// Path segments that imply an out-of-bound dependency even without a
    /// package alias.
    pub path_segments: &'a [&'a str],
    /// Exact names exempt from `name_prefixes` matches — engine infrastructure
    /// whose name collides with a forbidden prefix. Exemptions never override
    /// `exact_names`.
    pub exempt_names: &'a [&'a str],
    /// Exact workspace-relative paths exempt from `path_segments` matches.
    /// This is for intentionally shared contracts that live in an otherwise
    /// forbidden source tree. It never overrides a forbidden dependency name.
    pub exempt_paths: &'a [&'a str],
}

/// Which manifest dependency tables a boundary scan should inspect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// These independent flags intentionally describe a manifest-table scan mask;
// replacing them with state enums would make valid combinations harder to express.
#[allow(clippy::struct_excessive_bools)]
pub struct DependencyScan {
    pub include_dependencies: bool,
    pub include_build_dependencies: bool,
    pub include_dev_dependencies: bool,
    pub include_target_dependencies: bool,
    pub include_workspace_dependencies: bool,
}

impl DependencyScan {
    /// Production-facing crate boundaries include build-time dependencies but
    /// ignore test-only and workspace-root tables.
    pub const PRODUCTION: Self = Self {
        include_dependencies: true,
        include_build_dependencies: true,
        include_dev_dependencies: false,
        include_target_dependencies: true,
        include_workspace_dependencies: false,
    };

    /// Generated project manifests are edited by users and define their own
    /// workspace, so test-only and workspace-root dependencies are part of the
    /// policy surface.
    pub const PROJECT_WORKFLOW_MANIFEST: Self = Self {
        include_dependencies: true,
        include_build_dependencies: true,
        include_dev_dependencies: true,
        include_target_dependencies: true,
        include_workspace_dependencies: true,
    };
}

impl<'a> DependencyBoundary<'a> {
    #[must_use]
    pub const fn new(
        exact_names: &'a [&'a str],
        name_prefixes: &'a [&'a str],
        path_segments: &'a [&'a str],
    ) -> Self {
        Self {
            exact_names,
            name_prefixes,
            path_segments,
            exempt_names: &[],
            exempt_paths: &[],
        }
    }

    /// Exempt engine-infrastructure crate names from `name_prefixes` matches.
    #[must_use]
    pub const fn with_exempt_names(mut self, exempt_names: &'a [&'a str]) -> Self {
        self.exempt_names = exempt_names;
        self
    }

    /// Exempt exact workspace-relative paths from `path_segments` matches.
    #[must_use]
    pub const fn with_exempt_paths(mut self, exempt_paths: &'a [&'a str]) -> Self {
        self.exempt_paths = exempt_paths;
        self
    }

    #[must_use]
    pub fn name_is_forbidden(&self, name: &str) -> bool {
        if self.exact_names.contains(&name) {
            return true;
        }
        if self.exempt_names.contains(&name) {
            return false;
        }
        self.name_prefixes
            .iter()
            .any(|prefix| name.starts_with(prefix))
    }

    #[must_use]
    pub fn path_is_forbidden(&self, path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        let normalized = normalized.strip_prefix("./").unwrap_or(&normalized);
        if self.exempt_paths.iter().any(|exempt| {
            let exempt = exempt.replace('\\', "/");
            let exempt = exempt.strip_prefix("./").unwrap_or(&exempt);
            normalized == exempt
        }) {
            return false;
        }
        let segments = normalized
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != ".")
            .collect::<Vec<_>>();

        segments
            .iter()
            .any(|segment| self.name_is_forbidden(segment) || self.path_segments.contains(segment))
            || self.path_implies_forbidden_crate_name(&segments)
    }

    fn path_implies_forbidden_crate_name(&self, segments: &[&str]) -> bool {
        for (index, segment) in segments.iter().enumerate() {
            if *segment != "crates" {
                continue;
            }

            match segments.get(index + 1).copied() {
                Some("az") => {
                    if self.rest_path_implies_forbidden_crate("az", &segments[index + 2..]) {
                        return true;
                    }
                }
                Some("editor")
                    if self.editor_path_implies_forbidden_crate(&segments[index + 2..]) =>
                {
                    return true;
                }
                _ => {}
            }
        }

        false
    }

    fn rest_path_implies_forbidden_crate(&self, prefix: &str, rest: &[&str]) -> bool {
        if rest.is_empty() {
            return false;
        }

        let crate_name = format!("{prefix}-{}", rest.join("-"));
        self.name_is_forbidden(&crate_name)
    }

    fn editor_path_implies_forbidden_crate(&self, rest: &[&str]) -> bool {
        let Some(first) = rest.first().copied() else {
            return false;
        };

        if first == "core" && self.name_is_forbidden("az-editor") {
            return true;
        }

        self.rest_path_implies_forbidden_crate("az-editor", rest)
    }
}

/// Returns forbidden production dependencies from `manifest`, resolving
/// `{ workspace = true }` aliases through `workspace_manifest`.
///
/// The scan covers `[dependencies]`, `[build-dependencies]`, and matching
/// target-specific dependency tables. It intentionally ignores
/// `[dev-dependencies]` and `[workspace]` in the crate manifest because those do
/// not ship in the production boundary being guarded.
#[must_use]
pub fn forbidden_production_dependencies(
    manifest: &Value,
    workspace_manifest: &Value,
    boundary: DependencyBoundary<'_>,
) -> Vec<String> {
    forbidden_manifest_dependencies(
        manifest,
        workspace_manifest,
        boundary,
        DependencyScan::PRODUCTION,
    )
}

/// Returns forbidden dependencies from `manifest`, resolving
/// `{ workspace = true }` aliases through `workspace_manifest`, using the
/// dependency tables selected by `scan`.
#[must_use]
pub fn forbidden_manifest_dependencies(
    manifest: &Value,
    workspace_manifest: &Value,
    boundary: DependencyBoundary<'_>,
    scan: DependencyScan,
) -> Vec<String> {
    let mut violations = Vec::new();
    collect_scanned_dependency_violations(
        manifest,
        workspace_manifest,
        "",
        scan,
        &mut violations,
        &mut |section_path, alias, dependency, workspace_manifest, violations| {
            collect_forbidden_dependency(
                section_path,
                alias,
                dependency,
                workspace_manifest,
                boundary,
                violations,
            );
        },
    );
    violations.sort();
    violations
}

/// Returns production dependencies whose resolved crate/package name is not in
/// `allowed_names`, or whose direct/workspace path crosses `forbidden_paths`.
///
/// This uses the same manifest traversal and workspace-alias resolution as
/// [`forbidden_production_dependencies`], so allowlist guards do not drift into
/// a second TOML scanner.
#[must_use]
pub fn non_allowlisted_production_dependencies(
    manifest: &Value,
    workspace_manifest: &Value,
    allowed_names: &[&str],
    forbidden_paths: DependencyBoundary<'_>,
) -> Vec<String> {
    let mut violations = Vec::new();
    collect_scanned_dependency_violations(
        manifest,
        workspace_manifest,
        "",
        DependencyScan::PRODUCTION,
        &mut violations,
        &mut |section_path, alias, dependency, workspace_manifest, violations| {
            let package_name =
                resolved_dependency_package_name(alias, dependency, workspace_manifest);
            let path = resolved_dependency_path(alias, dependency, workspace_manifest);
            // proto→proto composition is legitimate ABI layering: any `az-proto-*` crate may depend on other `az-proto-*` crates without an explicit allowlist entry.
            let name_allowed =
                allowed_names.contains(&package_name) || package_name.starts_with("az-proto-");
            if !name_allowed || path.is_some_and(|path| forbidden_paths.path_is_forbidden(path)) {
                violations.push(format!(
                    "{section_path}.{alias}{}",
                    resolved_dependency_label(alias, dependency, workspace_manifest)
                ));
            }
        },
    );
    violations.sort();
    violations
}

/// A Rust source file with `#[cfg(test)] mod ...` blocks removed.
///
/// Architecture tests use this when scanning production source text for
/// forbidden service, process, database, or UI boundary usage. Keeping the
/// stripping rule here prevents each crate from growing a slightly different
/// definition of "production source".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductionSource {
    pub path: PathBuf,
    pub source: String,
}

/// Collect Rust source files below `root`, stripping test modules from each
/// file and sorting the result by path for deterministic diagnostics.
///
/// # Errors
///
/// Returns the underlying filesystem error when a directory cannot be read or
/// a Rust source file cannot be loaded.
pub fn collect_production_rust_sources(
    root: impl AsRef<Path>,
) -> io::Result<Vec<ProductionSource>> {
    let mut sources = Vec::new();
    collect_production_rust_sources_inner(root.as_ref(), &mut sources)?;
    sources.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(sources)
}

/// Remove `#[cfg(test)] mod ...` blocks while preserving other source text.
///
/// This deliberately does not strip individual `#[cfg(test)]` functions. Some
/// boundary tests need to assert that public harness APIs are explicitly gated,
/// and those assertions must still see the function signatures and attributes.
///
/// Files whose *declaration* carries the gate (`#[cfg(test)] mod tests;` in a
/// parent) have no marker inside the file itself. Such files start with the
/// sentinel comment `// test-only module`, which this function treats as
/// "entire file is test-only" and strips completely.
#[must_use]
pub fn production_source_without_cfg_test_modules(source: &str) -> String {
    if source.trim_start().starts_with("// test-only module") {
        return String::new();
    }
    let mut output = String::new();
    let mut pending_cfg_test = false;
    let mut skipping_cfg_test_block = false;
    let mut brace_depth = 0i32;

    for line in source.lines() {
        let trimmed = line.trim_start();
        if skipping_cfg_test_block {
            brace_depth += brace_delta(line);
            if brace_depth <= 0 && line.contains('}') {
                skipping_cfg_test_block = false;
                brace_depth = 0;
            }
            continue;
        }

        if pending_cfg_test {
            pending_cfg_test = false;
            if trimmed.starts_with("mod ") || trimmed.starts_with("pub mod ") {
                if !trimmed.ends_with(';') {
                    brace_depth = brace_delta(line).max(0);
                    if !line.contains('{') || brace_depth > 0 {
                        skipping_cfg_test_block = true;
                    }
                }
                continue;
            }

            output.push_str("#[cfg(test)]\n");
        }

        if trimmed == "#[cfg(test)]" {
            pending_cfg_test = true;
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    if pending_cfg_test {
        output.push_str("#[cfg(test)]\n");
    }

    output
}

/// Returns true when every matching public function signature is immediately
/// preceded by `#[cfg(test)]` or `#[cfg(any(test, feature = "test-support"))]`.
#[must_use]
pub fn public_functions_are_cfg_test_or_test_support(source: &str, signature: &str) -> bool {
    public_functions_have_cfg(
        source,
        signature,
        &[
            "#[cfg(test)]",
            "#[cfg(any(test, feature = \"test-support\"))]",
        ],
    )
}

/// Returns true when every line whose trimmed text starts with `signature` is
/// immediately preceded by one of `accepted_cfgs`.
///
/// It allows blank lines, doc comments, and attributes between the cfg and the
/// function signature.
#[must_use]
pub fn public_functions_have_cfg(source: &str, signature: &str, accepted_cfgs: &[&str]) -> bool {
    let lines = source.lines().collect::<Vec<_>>();
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.trim_start().starts_with(signature))
        .all(|(index, _)| {
            lines[..index]
                .iter()
                .rev()
                .take_while(|line| {
                    let trimmed = line.trim();
                    trimmed.is_empty() || trimmed.starts_with("#[") || trimmed.starts_with("///")
                })
                .any(|line| accepted_cfgs.contains(&line.trim()))
        })
}

fn collect_production_rust_sources_inner(
    root: &Path,
    sources: &mut Vec<ProductionSource>,
) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_production_rust_sources_inner(&path, sources)?;
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            let source = fs::read_to_string(&path)?;
            sources.push(ProductionSource {
                path,
                source: production_source_without_cfg_test_modules(&source),
            });
        }
    }
    Ok(())
}

fn brace_delta(line: &str) -> i32 {
    line.chars().fold(0, |depth, ch| match ch {
        '{' => depth + 1,
        '}' => depth - 1,
        _ => depth,
    })
}

fn collect_scanned_dependency_violations<F>(
    value: &Value,
    workspace_manifest: &Value,
    section_path: &str,
    scan: DependencyScan,
    violations: &mut Vec<String>,
    inspect: &mut F,
) where
    F: FnMut(&str, &str, &Value, &Value, &mut Vec<String>),
{
    let Some(table) = value.as_table() else {
        return;
    };

    for (key, value) in table {
        let child_path = if section_path.is_empty() {
            key.clone()
        } else {
            format!("{section_path}.{key}")
        };

        match key.as_str() {
            "dependencies" if scan.include_dependencies => {
                collect_dependency_violations(
                    value,
                    workspace_manifest,
                    &child_path,
                    violations,
                    inspect,
                );
            }
            "build-dependencies" if scan.include_build_dependencies => {
                collect_dependency_violations(
                    value,
                    workspace_manifest,
                    &child_path,
                    violations,
                    inspect,
                );
            }
            "dev-dependencies" if scan.include_dev_dependencies => {
                collect_dependency_violations(
                    value,
                    workspace_manifest,
                    &child_path,
                    violations,
                    inspect,
                );
            }
            "target" if scan.include_target_dependencies => collect_scanned_dependency_violations(
                value,
                workspace_manifest,
                &child_path,
                scan,
                violations,
                inspect,
            ),
            "dependencies" | "build-dependencies" | "dev-dependencies" | "target" => {}
            "workspace" => {
                if scan.include_workspace_dependencies
                    && let Some(dependencies) = value.get("dependencies")
                {
                    collect_dependency_violations(
                        dependencies,
                        workspace_manifest,
                        &format!("{child_path}.dependencies"),
                        violations,
                        inspect,
                    );
                }
            }
            _ => collect_scanned_dependency_violations(
                value,
                workspace_manifest,
                &child_path,
                scan,
                violations,
                inspect,
            ),
        }
    }
}

fn collect_dependency_violations<F>(
    value: &Value,
    workspace_manifest: &Value,
    section_path: &str,
    violations: &mut Vec<String>,
    inspect: &mut F,
) where
    F: FnMut(&str, &str, &Value, &Value, &mut Vec<String>),
{
    let Some(dependencies) = value.as_table() else {
        return;
    };

    for (alias, dependency) in dependencies {
        inspect(
            section_path,
            alias,
            dependency,
            workspace_manifest,
            violations,
        );
    }
}

fn collect_forbidden_dependency(
    section_path: &str,
    alias: &str,
    dependency: &Value,
    workspace_manifest: &Value,
    boundary: DependencyBoundary<'_>,
    violations: &mut Vec<String>,
) {
    let violation_start = violations.len();
    if boundary.name_is_forbidden(alias) {
        violations.push(format!("{section_path}.{alias}"));
    }
    if let Some(package) = dependency_package_name(dependency)
        && boundary.name_is_forbidden(package)
    {
        violations.push(format!("{section_path}.{alias}(package {package})"));
    }
    if let Some(package) = workspace_dependency_package_name(alias, dependency, workspace_manifest)
        && boundary.name_is_forbidden(package)
    {
        violations.push(format!(
            "{section_path}.{alias}(workspace package {package})"
        ));
    }
    if violations.len() == violation_start {
        if let Some(path) = dependency_path(dependency)
            && boundary.path_is_forbidden(path)
        {
            violations.push(format!("{section_path}.{alias}(path {path})"));
        }
        if let Some(path) = workspace_dependency_path(alias, dependency, workspace_manifest)
            && boundary.path_is_forbidden(path)
        {
            violations.push(format!("{section_path}.{alias}(workspace path {path})"));
        }
    }
}

fn resolved_dependency_package_name<'a>(
    alias: &'a str,
    dependency: &'a Value,
    workspace_manifest: &'a Value,
) -> &'a str {
    dependency_package_name(dependency)
        .or_else(|| workspace_dependency_package_name(alias, dependency, workspace_manifest))
        .unwrap_or(alias)
}

fn resolved_dependency_path<'a>(
    alias: &'a str,
    dependency: &'a Value,
    workspace_manifest: &'a Value,
) -> Option<&'a str> {
    dependency_path(dependency)
        .or_else(|| workspace_dependency_path(alias, dependency, workspace_manifest))
}

fn resolved_dependency_label(
    alias: &str,
    dependency: &Value,
    workspace_manifest: &Value,
) -> String {
    if let Some(package) = dependency_package_name(dependency)
        && package != alias
    {
        return format!("(package {package})");
    }
    if let Some(package) = workspace_dependency_package_name(alias, dependency, workspace_manifest)
        && package != alias
    {
        return format!("(workspace package {package})");
    }
    if let Some(path) = dependency_path(dependency) {
        return format!("(path {path})");
    }
    if let Some(path) = workspace_dependency_path(alias, dependency, workspace_manifest) {
        return format!("(workspace path {path})");
    }
    String::new()
}

fn dependency_package_name(dependency: &Value) -> Option<&str> {
    dependency.as_table()?.get("package")?.as_str()
}

fn dependency_path(dependency: &Value) -> Option<&str> {
    dependency.as_table()?.get("path")?.as_str()
}

fn workspace_dependency_package_name<'a>(
    alias: &str,
    dependency: &Value,
    workspace_manifest: &'a Value,
) -> Option<&'a str> {
    workspace_dependency(alias, dependency, workspace_manifest).and_then(dependency_package_name)
}

fn workspace_dependency_path<'a>(
    alias: &str,
    dependency: &Value,
    workspace_manifest: &'a Value,
) -> Option<&'a str> {
    workspace_dependency(alias, dependency, workspace_manifest).and_then(dependency_path)
}

fn workspace_dependency<'a>(
    alias: &str,
    dependency: &Value,
    workspace_manifest: &'a Value,
) -> Option<&'a Value> {
    if dependency
        .as_table()?
        .get("workspace")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return None;
    }

    workspace_manifest
        .get("workspace")?
        .get("dependencies")?
        .get(alias)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXACT: &[&str] = &["az-editor", "gridmate", "lyshine"];
    const PREFIXES: &[&str] = &["az-gem-", "cry-"];
    const PATH_SEGMENTS: &[&str] = &["examples", "formats", "gems", "projects"];

    fn boundary() -> DependencyBoundary<'static> {
        DependencyBoundary::new(EXACT, PREFIXES, PATH_SEGMENTS)
    }

    const CAUSAL_SESSION_SURFACES: [&str; 8] = [
        "crates/az/session/src/supervisor.rs",
        "crates/az/session/src/rpc.rs",
        "crates/az/session/src/transport.rs",
        "crates/az/sessiond/src/lib.rs",
        "crates/az/daemon/src/lib.rs",
        "crates/editor/core/src/session_supervisor.rs",
        "crates/editor/core/src/runtime_host.rs",
        "src/commands/session.rs",
    ];

    const CAUSAL_SESSION_FORBIDDEN: [&str; 12] = [
        "StartServicesRequestState",
        "start_services_requested",
        "with_service_requests",
        "with_shutdown_request",
        "SESSION_STATUS_WATCH_INTERVAL",
        "SESSIOND_START_POLL_MS",
        "SERVICE_SHUTDOWN_POLL_MS",
        "RUNTIME_HOST_READY_POLL_MS",
        "RUNTIME_HOST_START_POLL",
        "poll_interval",
        "ensure_stop_services_result_matches_request",
        "ensure_start_services_result_matches_request",
    ];

    const WAVE_TWO_SURFACES: [&str; 6] = [
        "crates/az/daemon/src/lib.rs",
        "crates/az/daemon/src/main.rs",
        "crates/az/rpc/src/lib.rs",
        "crates/editor/core/src/daemon_bootstrap.rs",
        "src/commands/daemon.rs",
        "src/commands/editor.rs",
    ];

    const WAVE_TWO_FORBIDDEN: [&str; 6] = [
        "BUILD_CANCEL_POLL_MS",
        "EDITOR_SERVICE_RUNNING_POLL_MS",
        "WINDOWS_NAMED_PIPE_BUSY_BACKOFF",
        "--interval-ms",
        "idle_ms",
        "AtomicBool",
    ];

    const SERVICE_IDENTITY_FORBIDDEN: [&str; 7] = [
        "Generation",
        "generation_scoped_endpoint",
        "bump_generation",
        "service_generation",
        "lease_epoch",
        "runtime_launch_generation",
        "catalog_publication_generations",
    ];

    /// Asserts the editor's asset-facing waits stay event-driven.
    ///
    /// Split out of `asset_processing_waits_remain_event_and_deadline_driven`;
    /// that guard covers the processing side, this one the editor side.
    fn assert_editor_asset_waits_stay_event_driven(workspace_root: &Path) {
        let editor_subscription = production_source_without_cfg_test_modules(
            &fs::read_to_string(
                workspace_root.join("crates/editor/core/src/asset_processor/install.rs"),
            )
            .expect("read editor asset-processor subscription"),
        );
        for forbidden in [
            "spawn_asset_processor_health_watch",
            "run_asset_processor_health_watch",
            "poll_asset_processor_activity",
            "Duration::from_secs(1)",
        ] {
            assert!(
                !editor_subscription.contains(forbidden),
                "editor asset-processor cadence returned: {forbidden}"
            );
        }
        let editor_event_sink = production_source_without_cfg_test_modules(
            &fs::read_to_string(
                workspace_root.join("crates/editor/core/src/asset_processor/event_sink.rs"),
            )
            .expect("read editor asset-processor event sink"),
        );
        assert!(
            editor_event_sink.contains("oneshot::Receiver<ServiceHealth>")
                && editor_event_sink.contains("oneshot::Receiver<String>")
                && editor_event_sink.contains("publish_initial")
                && editor_event_sink.contains("publish_terminal")
                && editor_subscription.contains("admit_initial")
                && editor_subscription.contains("publish_asset_processor_stream_terminal"),
            "editor asset-processor stream must expose initial health and terminal close edges"
        );

        let mannequin = production_source_without_cfg_test_modules(
            &fs::read_to_string(
                workspace_root.join("crates/editor/core/src/mannequin_animation.rs"),
            )
            .expect("read mannequin catalog controller"),
        );
        for forbidden in [
            "CATALOG_WATCH_INTERVAL",
            "animation_catalog_signature",
            "timer(",
        ] {
            assert!(
                !mannequin.contains(forbidden),
                "mannequin catalog cadence returned: {forbidden}"
            );
        }
        assert!(
            mannequin.contains("subscribe_animation_catalog_input_invalidation"),
            "mannequin catalog must park on its coalesced catalog-input invalidation"
        );
    }

    /// Records violations where a termination path stopped being event-driven.
    ///
    /// Split out of `remaining_wave_two_deadlines_stay_event_driven`; that guard
    /// covers daemon and RPC waits, this one the editor, supervision, and
    /// owned-process termination paths.
    fn collect_termination_path_violations(workspace_root: &Path, violations: &mut Vec<String>) {
        let editor_source = production_source_without_cfg_test_modules(
            &fs::read_to_string(workspace_root.join("src/commands/editor.rs"))
                .expect("read editor CLI source"),
        );
        if editor_source.contains("fn wait_for_editor_services_running") {
            violations.push(
                "editor service readiness must consume terminal start status without polling"
                    .to_string(),
            );
        }

        let supervision_source =
            fs::read_to_string(workspace_root.join("crates/az/service-supervision/src/lib.rs"))
                .expect("read service supervision source");
        let recorded_termination = supervision_source
            .split("fn platform_terminate_recorded_service_process(")
            .nth(2)
            .expect("find Linux recorded-process termination")
            .split("fn recorded_unix_process_program(")
            .next()
            .expect("find end of Linux recorded-process termination");
        if recorded_termination.contains("sleep(") || !recorded_termination.contains("wait_until(")
        {
            violations.push(
                "Linux recorded-process SIGTERM grace must race exact exit with one deadline"
                    .to_string(),
            );
        }

        let work_source = fs::read_to_string(workspace_root.join("crates/az/work/src/process.rs"))
            .expect("read owned-process source");
        let owner_termination = work_source
            .split("unsafe fn terminate_watched_group_and_exit(")
            .nth(1)
            .expect("find owner watcher termination")
            .split("fn create_cloexec_pipe(")
            .next()
            .expect("find end of owner watcher termination");
        if owner_termination.contains("sleep(") || owner_termination.contains("nanosleep") {
            violations.push(
                "owned-process SIGTERM grace must race exact exit without sleeping".to_string(),
            );
        }
    }

    /// Reports each `forbidden` identifier that has reappeared in any file
    /// listed in `surfaces`, with `what` naming the class being guarded.
    ///
    /// Several guards below enforce the same shape — a removed cadence or
    /// control surface must not come back — so the scan lives here once.
    fn reintroduced_identifiers(
        workspace_root: &Path,
        surfaces: &[&str],
        forbidden: &[&str],
        what: &str,
    ) -> Vec<String> {
        let mut violations = Vec::new();
        for relative_path in surfaces {
            let source = production_source_without_cfg_test_modules(
                &fs::read_to_string(workspace_root.join(relative_path))
                    .unwrap_or_else(|error| panic!("read {relative_path}: {error}")),
            );
            for identifier in forbidden {
                if source.contains(identifier) {
                    violations.push(format!("{relative_path}: {what} `{identifier}` returned"));
                }
            }
        }
        violations
    }

    /// The workspace root, derived from this crate's manifest directory.
    ///
    /// Every guard below scans files relative to it, so it is derived once
    /// here rather than re-walked in each test.
    fn guard_workspace_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("architecture guard crate lives under crates/az/architecture-guard")
    }

    #[test]
    fn exempt_names_skip_prefix_matches_but_never_exact_names() {
        let boundary = DependencyBoundary::new(&["az-gem-banned"], PREFIXES, PATH_SEGMENTS)
            .with_exempt_names(&["az-gem-contract", "az-gem-banned"]);

        // Infrastructure exemption beats the `az-gem-` prefix rule.
        assert!(!boundary.name_is_forbidden("az-gem-contract"));
        // Exact forbidden names always win, even when listed as exempt.
        assert!(boundary.name_is_forbidden("az-gem-banned"));
        // Real gem crates stay forbidden by prefix.
        assert!(boundary.name_is_forbidden("az-gem-vegetation"));
    }

    #[test]
    fn gem_infrastructure_exemption_is_name_scoped_not_prefix_scoped() {
        let boundary = DependencyBoundary::new(EXACT, PREFIXES, PATH_SEGMENTS)
            .with_exempt_names(GEM_INFRASTRUCTURE_DEPENDENCIES);

        assert!(!boundary.name_is_forbidden("az-gem-contract"));
        // A hypothetical crate extending the exempt name is still a gem name.
        assert!(boundary.name_is_forbidden("az-gem-contract-extra"));
    }

    #[test]
    fn exempt_paths_are_exact_and_separator_agnostic() {
        let boundary = boundary().with_exempt_paths(&["gems/auth"]);

        assert!(!boundary.path_is_forbidden("gems/auth"));
        assert!(!boundary.path_is_forbidden(r".\gems\auth"));
        assert!(boundary.path_is_forbidden("gems/auth-steam"));
        assert!(boundary.path_is_forbidden("projects/sample/gems/auth"));
    }

    #[test]
    fn compatibility_format_dependency_list_includes_renamed_legacy_asset_crates() {
        for crate_name in [
            "az-physics-assets",
            "lmbr-central-assets",
            "lyshine-canvas",
            "nv-cloth-assets",
            "texture-atlas-assets",
        ] {
            assert!(
                COMPATIBILITY_FORMAT_DEPENDENCIES.contains(&crate_name),
                "{crate_name} must stay in the shared compatibility-format guard list"
            );
        }
    }

    #[test]
    fn finds_aliases_packages_workspace_packages_and_paths() {
        let manifest: Value = toml::from_str(
            r#"
            [dependencies]
            editor = { package = "az-editor", path = "../az-editor" }
            legacy = { path = "../formats/legacy/cry-chunk-assets" }
            harmless = "1"

            [target.'cfg(windows)'.dependencies]
            net = { workspace = true }
            gem = { path = "../gems/lmbr-central" }

            [target.'cfg(unix)'.build-dependencies]
            world = { workspace = true }
            "#,
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str(
            r#"
            [workspace.dependencies]
            net = { package = "gridmate", path = "../gridmate" }
            world = { package = "sample-plugin", path = "../projects/sample-plugin" }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_production_dependencies(&manifest, &workspace_manifest, boundary()),
            vec![
                "dependencies.editor(package az-editor)",
                "dependencies.legacy(path ../formats/legacy/cry-chunk-assets)",
                "target.cfg(unix).build-dependencies.world(workspace path ../projects/sample-plugin)",
                "target.cfg(windows).dependencies.gem(path ../gems/lmbr-central)",
                "target.cfg(windows).dependencies.net(workspace package gridmate)",
            ]
        );
    }

    #[test]
    fn ignores_dev_dependencies_and_workspace_table_in_crate_manifest() {
        let manifest: Value = toml::from_str(
            r#"
            [dev-dependencies]
            editor = { package = "az-editor", path = "../az-editor" }

            [workspace.dependencies]
            world = { package = "sample-plugin", path = "../projects/sample-plugin" }
            "#,
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str("[workspace.dependencies]").unwrap();

        assert!(
            forbidden_production_dependencies(&manifest, &workspace_manifest, boundary())
                .is_empty()
        );
    }

    #[test]
    fn project_workflow_scan_includes_dev_target_and_workspace_dependencies() {
        let manifest: Value = toml::from_str(
            r#"
            [dependencies]
            editor = { package = "az-editor", path = "../crates/editor/core" }
            engine-path = { path = "../crates/az/engine" }
            harmless = "1"

            [dev-dependencies]
            legacy = { path = "../formats/legacy/cry-chunk-assets" }

            [workspace.dependencies]
            net = { package = "project-game", path = "../formats/legacy/project-game" }

            [target.'cfg(windows)'.dependencies]
            net = { workspace = true }
            ui = { package = "lyshine", path = "../gems/lyshine" }
            "#,
        )
        .unwrap();
        let boundary = DependencyBoundary::new(
            &["az-editor", "az-engine", "lyshine"],
            &["cry-", "project-"],
            &[],
        );

        assert_eq!(
            forbidden_manifest_dependencies(
                &manifest,
                &manifest,
                boundary,
                DependencyScan::PROJECT_WORKFLOW_MANIFEST,
            ),
            vec![
                "dependencies.editor(package az-editor)",
                "dependencies.engine-path(path ../crates/az/engine)",
                "dev-dependencies.legacy(path ../formats/legacy/cry-chunk-assets)",
                "target.cfg(windows).dependencies.net(workspace package project-game)",
                "target.cfg(windows).dependencies.ui(package lyshine)",
                "workspace.dependencies.net(package project-game)",
            ]
        );
    }

    #[test]
    fn reports_workspace_path_when_package_name_is_allowed() {
        let manifest: Value = toml::from_str(
            r"
            [dependencies]
            compat = { workspace = true }
            ",
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str(
            r#"
            [workspace.dependencies]
            compat = { path = "../formats/legacy/safe-name" }
            "#,
        )
        .unwrap();

        assert_eq!(
            forbidden_production_dependencies(&manifest, &workspace_manifest, boundary()),
            vec!["dependencies.compat(workspace path ../formats/legacy/safe-name)"]
        );
    }

    #[test]
    fn derives_forbidden_crate_names_from_clean_source_tree_paths() {
        let manifest: Value = toml::from_str(
            r#"
            [dependencies]
            db = { path = "../crates/az/assetdb" }
            ui = { path = "../crates/editor/ui" }
            shell = { path = "../crates/editor/core" }
            proto = { path = "../crates/az/proto/core" }
            "#,
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str("[workspace.dependencies]").unwrap();

        let boundary = DependencyBoundary::new(
            &["az-assetdb", "az-editor", "az-editor-ui", "az-proto-core"],
            &[],
            &[],
        );

        assert_eq!(
            forbidden_production_dependencies(&manifest, &workspace_manifest, boundary),
            vec![
                "dependencies.db(path ../crates/az/assetdb)",
                "dependencies.proto(path ../crates/az/proto/core)",
                "dependencies.shell(path ../crates/editor/core)",
                "dependencies.ui(path ../crates/editor/ui)",
            ]
        );
    }

    #[test]
    fn allowlist_guard_uses_same_workspace_and_path_resolution() {
        let manifest: Value = toml::from_str(
            r#"
            [dependencies]
            capnp = "0.25"
            serde-json = "1"
            editor = { package = "az-editor-ui", path = "../crates/editor/ui" }
            legacy = { path = "../formats/legacy/cry-chunk-assets" }

            [target.'cfg(windows)'.dependencies]
            runtime = { workspace = true }
            compatible = { workspace = true }
            "#,
        )
        .unwrap();
        let workspace_manifest: Value = toml::from_str(
            r#"
            [workspace.dependencies]
            runtime = { package = "az-runtime-host", path = "../crates/az/runtime-host" }
            compatible = { path = "../formats/compat/az-pak" }
            "#,
        )
        .unwrap();
        let forbidden_paths = DependencyBoundary::new(
            &["az-editor-ui", "az-runtime-host"],
            &["cry-"],
            &["formats"],
        );

        assert_eq!(
            non_allowlisted_production_dependencies(
                &manifest,
                &workspace_manifest,
                &["capnp", "compatible", "runtime"],
                forbidden_paths,
            ),
            vec![
                "dependencies.editor(package az-editor-ui)",
                "dependencies.legacy(path ../formats/legacy/cry-chunk-assets)",
                "dependencies.serde-json",
                "target.cfg(windows).dependencies.compatible(workspace path ../formats/compat/az-pak)",
                "target.cfg(windows).dependencies.runtime(workspace package az-runtime-host)",
            ]
        );
    }

    #[test]
    fn production_source_scanner_removes_cfg_test_modules_only() {
        let source = r"
pub fn production_api() {}

#[cfg(test)]
mod tests {
    pub fn in_process_harness() {}
}

#[cfg(test)]
pub fn gated_helper() {}

#[cfg(test)]
mod architecture_tests;

pub fn second_production_api() {}
";

        let production = production_source_without_cfg_test_modules(source);

        assert!(production.contains("pub fn production_api()"));
        assert!(production.contains("pub fn second_production_api()"));
        assert!(production.contains("#[cfg(test)]\npub fn gated_helper()"));
        assert!(!production.contains("in_process_harness"));
        assert!(!production.contains("mod architecture_tests"));
    }

    #[test]
    fn azoth_owned_versions_stay_at_one_until_release_boundary() {
        let workspace_root = guard_workspace_root();
        let mut violations = Vec::new();

        for relative_root in ["crates/az", "crates/editor"] {
            collect_owned_version_violations(
                &workspace_root.join(relative_root),
                workspace_root,
                &mut violations,
            )
            .expect("scan Rust source versions");
        }

        violations.sort();
        assert!(
            violations.is_empty(),
            "Azoth-owned schema/format/protocol versions must stay pinned to 1 before a release boundary.\nExternal spec/protocol values need an explicit allowance in this guard.\n{}",
            violations.join("\n")
        );
    }

    fn collect_owned_version_violations(
        root: &Path,
        workspace_root: &Path,
        violations: &mut Vec<String>,
    ) -> io::Result<()> {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_owned_version_violations(&path, workspace_root, violations)?;
                continue;
            }

            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }

            let source = production_source_without_cfg_test_modules(&fs::read_to_string(&path)?);
            let relative_path = path
                .strip_prefix(workspace_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");

            for (index, line) in source.lines().enumerate() {
                if line_has_owned_version_literal_above_one(line)
                    && !allowed_external_version_literal(&relative_path, line)
                {
                    violations.push(format!("{}:{}: {}", relative_path, index + 1, line.trim()));
                }

                if schema_catalog_static_version_above_one(line) {
                    violations.push(format!("{}:{}: {}", relative_path, index + 1, line.trim()));
                }
            }
        }

        Ok(())
    }

    fn line_has_owned_version_literal_above_one(line: &str) -> bool {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('*') {
            return false;
        }

        let lower = trimmed.to_ascii_lowercase();
        if !lower.contains("version") {
            return false;
        }

        direct_version_literal_above_one(trimmed, &lower)
    }

    fn schema_catalog_static_version_above_one(line: &str) -> bool {
        for marker in [
            "SchemaCatalog::from_static(",
            "SchemaCatalog::try_from_static(",
        ] {
            let Some(start) = line.find(marker) else {
                continue;
            };
            let tail = &line[start + marker.len()..];
            let digits = tail
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if digits.parse::<u64>().is_ok_and(|value| value > 1) {
                return true;
            }
        }

        false
    }

    fn direct_version_literal_above_one(line: &str, lower: &str) -> bool {
        if lower.contains("const") && literal_after_separator_above_one(line, '=') {
            return true;
        }

        [
            "version:",
            "version =",
            "version=",
            ".version,",
            "set_version(",
        ]
        .iter()
        .any(|marker| literal_after_marker_above_one(line, lower, marker))
    }

    fn literal_after_marker_above_one(line: &str, lower: &str, marker: &str) -> bool {
        let mut search_from = 0;
        while let Some(offset) = lower[search_from..].find(marker) {
            let marker_end = search_from + offset + marker.len();
            search_from = marker_end;

            let digits = line[marker_end..]
                .trim_start()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if digits.parse::<u64>().is_ok_and(|value| value > 1) {
                return true;
            }
        }

        false
    }

    fn literal_after_separator_above_one(line: &str, separator: char) -> bool {
        let Some(separator_index) = line.rfind(separator) else {
            return false;
        };
        let digits = line[separator_index + separator.len_utf8()..]
            .trim_start()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>();
        digits.parse::<u64>().is_ok_and(|value| value > 1)
    }

    fn allowed_external_version_literal(relative_path: &str, line: &str) -> bool {
        // Compile-fail fixtures intentionally exercise malformed/non-v1
        // Prefab migration declarations; they are not owned source formats.
        relative_path.starts_with("crates/az/prefab-derive/tests/ui/")
            // Mesh is the first real Prefab migration-chain fixture. Its v2
            // source registration and retained legacy migration are required
            // by ADR 0022 phase 3 while all unrelated owned versions stay v1.
            || (relative_path == "crates/az/render/src/mesh.rs"
                && line.to_ascii_lowercase().contains("version"))
            // This versions the internal projected registry snapshot, not an
            // authored asset schema or runtime product.
            || (relative_path == "crates/az/proto/project/src/lib.rs"
                && line.contains("SCHEMA_CATALOG_SNAPSHOT_VERSION"))
            // AZSCENE_FORMAT_VERSION needs no allowance and has none. The
            // codec was rebuilt and the constant reset to 1 (the same change
            // renamed the fixture scene_v3.rs -> scene_v1.rs), so the pin at 1
            // is the guard's default and nothing here has to say so. A bespoke
            // arm used to match the constant's *name*, which would have
            // swallowed a future bump silently; if AZSCENE is ever raised, the
            // bump must be recorded as a pinned line in
            // `acknowledged_owned_version` like every other raised version.
            || matches!(
                relative_path,
                // This source file is linked only through a `#[cfg(test)]` module;
                // the scanner cannot infer that from the standalone file.
                "crates/az/project-host/src/migration_tests.rs"
                    // Editor-local preference database migration version. This is
                    // neither an authored asset schema nor a runtime wire format.
                    | "crates/editor/core/src/project_manager_preferences/schema.rs"
                    | "crates/az/asset/src/package_payload.rs"
                    | "crates/az/gridmate/src/carrier/connection_security.rs"
                    | "crates/az/gridmate/src/driver/test_cert.rs"
                    // Integration test under `tests/`, so the whole file is
                    // test-only and there is no `#[cfg(test)]` module for the
                    // scanner to strip. Its `version = 4` lines are Cargo's
                    // external lockfile format version inside `Cargo.lock`
                    // fixture strings, not an Azoth-owned version.
                    | "crates/az/project/tests/role_lock_validation.rs"
            ) || (relative_path == "crates/az/gridmate/src/carrier/carrier_desc.rs"
                && line.contains("desc.version")
                && line.contains(", 5"))
            // Negative test: asserts the codec REJECTS a forged newer-version
            // header, which is enforcement of the pin, not a bump.
            || (relative_path == "crates/az/terrain-runtime/src/codec.rs"
                && line.contains("TerrainAssetCodecError::UnsupportedVersion"))
            || acknowledged_owned_version(relative_path, line)
    }

    /// Narrow, reviewed exceptions to the pre-release pin at 1.
    ///
    /// The governing rule is the one this test is named for: Azoth has not
    /// released, so every owned schema/format/protocol version stays at 1.
    /// Pre-release, the correct answer to an incompatible format change is to
    /// change the format and reprocess every artifact — not to increment a
    /// gate. Incrementing only starts to mean something at the release
    /// boundary, once an artifact can exist that its producer cannot reach.
    ///
    /// So this table is an escape hatch, not the normal path, and a bump is
    /// not self-justifying: "someone raised it deliberately" is not evidence
    /// that it must be above 1. An entry belongs here only when the value
    /// cannot be 1 for a reason outside this repo's control, or when it is
    /// not a compatibility gate at all (a machine-local regeneration stamp).
    /// No on-disk or wire format gate is currently listed, and that is the
    /// intended steady state — prefer emptying this table to growing it.
    ///
    /// Each entry pins the whole acknowledged line, value included, so raising
    /// any of these again re-trips this guard until the new value is reviewed
    /// and recorded here. This records reality; it does not relax the rule.
    fn acknowledged_owned_version(relative_path: &str, line: &str) -> bool {
        const ACKNOWLEDGED: &[(&str, &str)] = &[
            // Freshness stamp for the generated `.cargo/config.toml` body
            // (ADR 0029). A machine-local regeneration trigger, the same class
            // as the editor preference database version allowed above — not an
            // authored asset schema and not a runtime wire format.
            (
                "crates/az/project/src/patch_table.rs",
                "pub const ENGINE_PATCH_CONFIG_FORMAT_VERSION: u32 = 8;",
            ),
            // ADR 0036's generated-target schema is explicit because generated
            // workspaces are regenerated, never migrated. Schema 9 records a
            // typed process kind and restores runtime-host as a statically
            // composed independent service workspace after the real-DLL proof
            // invalidated ADR 0040's wide-Rust prebuilt-host boundary. The
            // runtime-host load manifest is removed in the same cutover, so
            // each role still has one composition authority. This entry
            // acknowledges the emitted value and re-trips on any future bump.
            (
                "crates/az/project/src/target_generation.rs",
                "pub const GENERATED_TARGETS_SCHEMA_VERSION: u32 = 9;",
            ),
        ];

        ACKNOWLEDGED
            .iter()
            .any(|(path, text)| *path == relative_path && line.trim() == *text)
    }

    #[test]
    fn service_generations_stay_deleted_and_run_stays_observational() {
        let workspace_root = guard_workspace_root();
        let service_source_roots = [
            "crates/az/proto/core/src",
            "crates/az/proto/session/src",
            "crates/az/proto/runtime/src",
            "crates/az/proto/observability/src",
            "crates/az/proto/project/src",
            "crates/az/service-supervision/src",
            "crates/az/service-catalog/src",
            "crates/az/service-entrypoint/src",
            "crates/az/session/src",
            "crates/az/sessiond/src",
            "crates/az/daemon/src",
            "crates/az/asset-processor/src",
            "crates/az/asset-processor-host/src",
            "crates/az/asset-worker/src",
            "crates/az/project-host/src",
            "crates/az/runtime-host/src",
            "crates/az/observability/src",
            "crates/az/observability-control/src",
            "src",
        ];
        let editor_source_roots = [
            "crates/editor/core/src",
            "crates/editor/inspector/src",
            "crates/editor/ui/src",
        ];
        let service_schemas = [
            "crates/az/proto/core/schema/azoth/core.capnp",
            "crates/az/proto/session/schema/azoth/session.capnp",
            "crates/az/proto/runtime/schema/azoth/runtime.capnp",
            "crates/az/proto/observability/schema/azoth/observability.capnp",
            "crates/az/proto/project/schema/azoth/project.capnp",
        ];
        let mut violations = Vec::new();

        for relative_path in service_schemas {
            let source = fs::read_to_string(workspace_root.join(relative_path))
                .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));
            if source_identifier_present(&source, "generation") {
                violations.push(format!(
                    "{relative_path}: service wire schema reintroduced `generation`"
                ));
            }
        }

        for relative_path in [
            "crates/az/daemon/src/transport.rs",
            "crates/az/session/src/transport.rs",
        ] {
            let source = production_source_without_cfg_test_modules(
                &fs::read_to_string(workspace_root.join(relative_path))
                    .unwrap_or_else(|error| panic!("read {relative_path}: {error}")),
            );
            if source.contains("remove_file(") {
                violations.push(format!(
                    "{relative_path}: leaf RPC transport may not unlink a stable Unix endpoint"
                ));
            }
        }
        for relative_path in [
            "crates/az/asset-processor/src/transport.rs",
            "crates/az/daemon/src/transport.rs",
            "crates/az/observability-control/src/transport.rs",
            "crates/az/project-host/src/transport.rs",
            "crates/az/runtime-host/src/transport.rs",
            "crates/az/session/src/transport.rs",
        ] {
            let source = production_source_without_cfg_test_modules(
                &fs::read_to_string(workspace_root.join(relative_path))
                    .unwrap_or_else(|error| panic!("read {relative_path}: {error}")),
            );
            if !source.contains("first_pipe_instance(true)") {
                violations.push(format!(
                    "{relative_path}: first Windows named-pipe listener must refuse an existing owner"
                ));
            }
        }

        for relative_root in service_source_roots {
            collect_service_identity_violations(
                &workspace_root.join(relative_root),
                workspace_root,
                true,
                &mut violations,
            )
            .unwrap_or_else(|error| panic!("scan {relative_root}: {error}"));
        }
        for relative_root in editor_source_roots {
            collect_service_identity_violations(
                &workspace_root.join(relative_root),
                workspace_root,
                false,
                &mut violations,
            )
            .unwrap_or_else(|error| panic!("scan {relative_root}: {error}"));
        }

        violations.sort();
        assert!(
            violations.is_empty(),
            "service launch generations must stay deleted, and `run` may only be compared for transport validity or retained-log selection:\n{}",
            violations.join("\n")
        );
    }

    #[test]
    fn completed_session_lifecycle_surfaces_stay_causal() {
        let workspace_root = guard_workspace_root();
        let surfaces = CAUSAL_SESSION_SURFACES;
        let forbidden = CAUSAL_SESSION_FORBIDDEN;
        let violations = reintroduced_identifiers(
            workspace_root,
            &surfaces,
            &forbidden,
            "removed cadence/atomic control surface",
        );
        assert!(
            violations.is_empty(),
            "session lifecycle control must remain causal and terminal:\n{}",
            violations.join("\n")
        );

        let editor_source =
            fs::read_to_string(workspace_root.join("crates/editor/core/src/session_supervisor.rs"))
                .expect("read editor session-supervisor source");
        let editor_shutdown = editor_source
            .split("pub async fn shutdown_supervisor(")
            .nth(1)
            .expect("find editor client shutdown boundary")
            .split("    async fn subscribe_status_events(")
            .next()
            .expect("find end of editor client shutdown boundary");
        assert!(
            !editor_shutdown.contains(".promise.await"),
            "editor shutdownSupervisor must be one-way after terminal stop"
        );
        let editor_stop = editor_source
            .split("fn spawn_session_services_stop(")
            .nth(1)
            .expect("find editor stop workflow")
            .split("fn spawn_session_services_start(")
            .next()
            .expect("find end of editor stop workflow");
        let editor_terminal_stop = editor_stop
            .find(".stop_services(")
            .expect("editor stop workflow must request terminal stop");
        let editor_one_way_shutdown = editor_stop
            .find(".shutdown_supervisor(")
            .expect("editor stop workflow must request supervisor shutdown");
        assert!(
            editor_terminal_stop < editor_one_way_shutdown,
            "editor must receive the terminal stop result before one-way supervisor shutdown"
        );

        let cli_source = fs::read_to_string(workspace_root.join("src/commands/session.rs"))
            .expect("read session CLI source");
        let cli_shutdown = cli_source
            .split("fn request_session_supervisor_shutdown(")
            .nth(1)
            .expect("find CLI shutdown boundary")
            .split("fn request_session_service_start_for_proto_manifest(")
            .next()
            .expect("find end of CLI shutdown boundary");
        assert!(
            !cli_shutdown.contains(".promise.await"),
            "CLI shutdownSupervisor must be one-way after terminal stop"
        );
        assert!(
            cli_shutdown.contains("connect_twoparty_bootstrap_scoped")
                && cli_shutdown.contains("connection.disconnect()"),
            "short-lived CLI shutdown must gracefully flush its scoped RPC connection"
        );
        let cli_stop = cli_source
            .split("pub fn stop_services(")
            .nth(1)
            .expect("find CLI stop workflow")
            .split("fn run_session_mutation_with_supervisor_shutdown(")
            .next()
            .expect("find end of CLI stop workflow");
        let cli_terminal_stop = cli_stop
            .find("request_session_service_shutdown(")
            .expect("CLI stop workflow must request terminal stop");
        let cli_one_way_shutdown = cli_stop
            .find("request_session_supervisor_shutdown(")
            .expect("CLI stop workflow must request supervisor shutdown");
        assert!(
            cli_terminal_stop < cli_one_way_shutdown,
            "CLI must receive the terminal stop result before one-way supervisor shutdown"
        );

        let daemon_source = fs::read_to_string(workspace_root.join("crates/az/daemon/src/lib.rs"))
            .expect("read daemon source");
        let daemon_shutdown = daemon_source
            .split("fn request_session_service_stop(")
            .nth(1)
            .expect("find daemon stop workflow")
            .split("fn run_session_supervisor_rpc<")
            .next()
            .expect("find end of daemon stop workflow");
        let daemon_terminal_shutdown = daemon_shutdown
            .split("shutdown_supervisor_request()")
            .nth(1)
            .expect("find daemon terminal shutdown call");
        assert!(
            daemon_shutdown.contains("connect_session_supervisor_scoped")
                && daemon_terminal_shutdown.contains("connection.disconnect()")
                && !daemon_terminal_shutdown.contains(".promise.await"),
            "short-lived daemon shutdown must flush a scoped connection without awaiting a reply"
        );
    }

    #[test]
    fn asset_processing_waits_remain_event_and_deadline_driven() {
        let workspace_root = guard_workspace_root();
        let watcher = production_source_without_cfg_test_modules(
            &fs::read_to_string(workspace_root.join("crates/az/asset-processor/src/watcher.rs"))
                .expect("read asset source watcher"),
        );
        for forbidden in [
            "SOURCE_WATCH_REFRESH_INTERVAL",
            "SOURCE_WATCH_LOOP_TICK",
            "recv_timeout(",
        ] {
            assert!(
                !watcher.contains(forbidden),
                "asset source watcher cadence returned: {forbidden}"
            );
        }

        let cook = production_source_without_cfg_test_modules(
            &fs::read_to_string(workspace_root.join("src/commands/session.rs"))
                .expect("read session cook command"),
        );
        assert!(
            !cook.contains("BUILD_ASSET_PROCESSING_POLL_INTERVAL"),
            "asset-processing CLI polling returned"
        );
        assert!(
            cook.contains("wait_for_idle_request"),
            "asset-processing CLI must park on waitForIdle"
        );

        let log_follow = production_source_without_cfg_test_modules(
            &fs::read_to_string(workspace_root.join("src/commands/log_follow.rs"))
                .expect("read local log-follow change source"),
        );
        assert!(
            log_follow.contains("notify::recommended_watcher")
                && log_follow.contains("RecursiveMode::NonRecursive"),
            "local log following must park on a parent-directory file watch"
        );
        for forbidden in [
            "SERVICE_LOG_FOLLOW_POLL_MS",
            "thread::sleep",
            "recv_timeout(",
        ] {
            assert!(
                !log_follow.contains(forbidden),
                "local log-follow cadence returned: {forbidden}"
            );
        }

        let plain_follow = cook
            .split("fn follow_log_file_from(")
            .nth(1)
            .expect("find plain log follower")
            .split("fn drain_log_file_from(")
            .next()
            .expect("find end of plain log follower");
        let structured_follow = cook
            .split("fn follow_structured_log_file_from(")
            .nth(1)
            .expect("find structured log follower")
            .split("fn trim_log_line_end(")
            .next()
            .expect("find end of structured log follower");
        for (label, follower) in [("plain", plain_follow), ("structured", structured_follow)] {
            assert!(
                follower.contains("FileChangeEvents::new") && follower.contains("events.wait()"),
                "{label} log follower must consume local filesystem edges"
            );
            assert!(
                !follower.contains("request_service_log_chunk") && !follower.contains("sleep("),
                "{label} log follower may not poll through RPC or a timer"
            );
        }

        assert_editor_asset_waits_stay_event_driven(workspace_root);
    }

    #[test]
    fn blocking_pipeline_cancellation_stays_event_driven() {
        let workspace_root = guard_workspace_root();
        let relative_path = "crates/az/jobs/src/pipeline.rs";
        let source = production_source_without_cfg_test_modules(
            &fs::read_to_string(workspace_root.join(relative_path))
                .unwrap_or_else(|error| panic!("read {relative_path}: {error}")),
        );
        for forbidden in ["CHANNEL_POLL_INTERVAL", "send_timeout", "recv_timeout"] {
            assert!(
                !source.contains(forbidden),
                "blocking pipeline cancellation polling returned: {forbidden}"
            );
        }
    }

    #[test]
    fn remaining_wave_two_deadlines_stay_event_driven() {
        let workspace_root = guard_workspace_root();
        let surfaces = WAVE_TWO_SURFACES;
        let forbidden = WAVE_TWO_FORBIDDEN;
        let mut violations = reintroduced_identifiers(
            workspace_root,
            &surfaces,
            &forbidden,
            "removed lifecycle cadence",
        );

        let daemon_source = fs::read_to_string(workspace_root.join("crates/az/daemon/src/lib.rs"))
            .expect("read daemon source");
        let build_loop = daemon_source
            .split("fn wait_for_build_command_exit(")
            .nth(1)
            .expect("find daemon build command loop")
            .split("fn run_project_build_command_with_capture(")
            .next()
            .expect("find end of daemon build command loop");
        for forbidden in ["try_wait", "recv_timeout", "thread::sleep"] {
            if build_loop.contains(forbidden) {
                violations.push(format!(
                    "daemon build command loop reintroduced `{forbidden}` cadence"
                ));
            }
        }
        for required in [
            "channel::select!",
            "recv(output_events)",
            "cancellation.receiver()",
            "lifecycle.receiver()",
        ] {
            if !build_loop.contains(required) {
                violations.push(format!(
                    "daemon build command loop lost causal source `{required}`"
                ));
            }
        }

        let rpc_source = fs::read_to_string(workspace_root.join("crates/az/rpc/src/lib.rs"))
            .expect("read RPC transport source");
        let named_pipe_connect = rpc_source
            .split("async fn connect_named_pipe_client_scoped<T>(")
            .nth(1)
            .expect("find Windows named-pipe client")
            .split("fn wait_named_pipe_available(")
            .next()
            .expect("find end of Windows named-pipe client");
        if !named_pipe_connect.contains("wait_named_pipe_available")
            || named_pipe_connect.contains("sleep(")
        {
            violations.push(
                "Windows named-pipe busy handling must use WaitNamedPipeW without sleep backoff"
                    .to_string(),
            );
        }

        for relative_path in [
            "crates/editor/core/src/daemon_bootstrap.rs",
            "src/commands/daemon.rs",
        ] {
            let source = fs::read_to_string(workspace_root.join(relative_path))
                .unwrap_or_else(|error| panic!("read {relative_path}: {error}"));
            let bootstrap = source
                .split("fn wait_for_daemon_start(")
                .nth(1)
                .unwrap_or_else(|| panic!("find daemon bootstrap in {relative_path}"))
                .split("fn spawn_daemon_process(")
                .next()
                .expect("find end of daemon bootstrap");
            if !bootstrap.contains("subscribe_ready")
                || !bootstrap.contains("wait_until(deadline)")
                || bootstrap.contains("try_wait")
                || bootstrap.contains("thread::sleep")
            {
                violations.push(format!(
                    "{relative_path}: daemon bootstrap must race record changes, exact exit, and one deadline"
                ));
            }
        }

        collect_termination_path_violations(workspace_root, &mut violations);

        assert!(
            violations.is_empty(),
            "completed Wave-2 lifecycle surfaces must remain event-driven:\n{}",
            violations.join("\n")
        );
    }

    /// Records observational service types that derive structural equality.
    ///
    /// Those types carry a `run` label that must not participate in control
    /// comparisons, so equality has to be written by hand.
    fn collect_observational_equality_violations(
        relative_path: &str,
        source: &str,
        violations: &mut Vec<String>,
    ) {
        for type_name in observational_service_types(relative_path) {
            if struct_derives_trait(source, type_name, "PartialEq")
                || struct_derives_trait(source, type_name, "Eq")
            {
                violations.push(format!(
                    "{relative_path}: `{type_name}` must define control equality explicitly so observational `run` fields cannot participate"
                ));
            }
        }
    }

    fn collect_service_identity_violations(
        root: &Path,
        workspace_root: &Path,
        reject_generation_identifier: bool,
        violations: &mut Vec<String>,
    ) -> io::Result<()> {
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_service_identity_violations(
                    &path,
                    workspace_root,
                    reject_generation_identifier,
                    violations,
                )?;
                continue;
            }
            if path.extension().and_then(|extension| extension.to_str()) != Some("rs") {
                continue;
            }

            let relative_path = path
                .strip_prefix(workspace_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let source = production_source_without_cfg_test_modules(&fs::read_to_string(&path)?);
            collect_observational_equality_violations(&relative_path, &source, violations);
            let publication_comparison_calls = source.matches(".same_durable_publication(").count();
            if relative_path == "crates/az/session/src/manager.rs" {
                if publication_comparison_calls != 1 {
                    violations.push(format!(
                        "{relative_path}: expected one durable-publication observation comparison, found {publication_comparison_calls}"
                    ));
                }
            } else if publication_comparison_calls != 0 {
                violations.push(format!(
                    "{relative_path}: durable publication equality cannot be used as a service control predicate"
                ));
            }
            let forbidden_identifiers = SERVICE_IDENTITY_FORBIDDEN;
            for identifier in forbidden_identifiers {
                if source_identifier_present(&source, identifier) {
                    violations.push(format!(
                        "{relative_path}: reintroduced removed service identity `{identifier}`"
                    ));
                }
            }
            // Builder-catalog generations are domain state, not service identity.
            if reject_generation_identifier
                && relative_path != "crates/az/asset-processor/src/catalog.rs"
                && source_identifier_present(&source, "generation")
            {
                violations.push(format!(
                    "{relative_path}: reintroduced a service-layer `generation` identifier"
                ));
            }

            let mut skeleton = symbol_skeleton(&source);
            for allowed in [
                "descriptor.run == Uuid::nil()",
                "process.run == Uuid::nil()",
                "run == Uuid::nil()",
                "result.run == Uuid::nil()",
                "health.run == Uuid::nil()",
                "record.run == Uuid::nil()",
                "descriptor.run == uuid::Uuid::nil()",
            ] {
                skeleton = skeleton.replace(&symbol_skeleton(allowed), " ");
            }
            if relative_path == "crates/az/session/src/rpc.rs" {
                for allowed in [
                    "Some(run) if run == process.run",
                    "process.previous_run == Some(run)",
                ] {
                    skeleton = skeleton.replace(&symbol_skeleton(allowed), " ");
                }
            }
            if relative_path == "crates/az/service-supervision/src/lib.rs" {
                let publication_comparison = symbol_skeleton("self.run == other.run");
                let publication_comparison_count =
                    skeleton.matches(&publication_comparison).count();
                if publication_comparison_count != 1 {
                    violations.push(format!(
                        "{relative_path}: expected exactly one explicit run comparison at the durable publication boundary, found {publication_comparison_count}"
                    ));
                }
                skeleton = skeleton.replacen(&publication_comparison, " ", 1);
            }
            if relative_path == "crates/editor/core/src/session_supervisor.rs" {
                skeleton = skeleton.replace(&symbol_skeleton("result.run != expected_run"), " ");
            }
            if relative_path == "src/commands/session.rs" {
                for allowed in [
                    "result.run != expected_run",
                    "Some(run) if run == process.run",
                    "process.previous_run == Some(run)",
                ] {
                    skeleton = skeleton.replace(&symbol_skeleton(allowed), " ");
                }
            }
            for comparison in run_comparison_contexts(&skeleton) {
                violations.push(format!(
                    "{relative_path}: launch `run` used in comparison `{comparison}`"
                ));
            }
        }

        Ok(())
    }

    fn source_identifier_present(source: &str, identifier: &str) -> bool {
        symbol_skeleton(source)
            .split_whitespace()
            .any(|token| token == identifier)
    }

    fn observational_service_types(relative_path: &str) -> &'static [&'static str] {
        match relative_path {
            "crates/az/proto/core/src/lib.rs" => &["ServiceDescriptor"],
            "crates/az/service-supervision/src/lib.rs" => {
                &["ServiceRecord", "ServiceProcessRecord"]
            }
            _ => &[],
        }
    }

    fn struct_derives_trait(source: &str, type_name: &str, trait_name: &str) -> bool {
        let declaration = format!("pub struct {type_name}");
        let Some(declaration_start) = source.find(&declaration) else {
            return false;
        };
        let prefix = &source[..declaration_start];
        let Some(derive_start) = prefix.rfind("#[derive(") else {
            return false;
        };
        let derive = &prefix[derive_start..];
        let Some(derive_end) = derive.find(")]") else {
            return false;
        };
        let between = &derive[derive_end + 2..];
        if between.contains("struct ") || between.contains("enum ") || between.contains("union ") {
            return false;
        }
        symbol_skeleton(&derive[..derive_end])
            .split_whitespace()
            .any(|token| token == trait_name)
    }

    fn run_comparison_contexts(skeleton: &str) -> Vec<String> {
        let tokens = skeleton.split_whitespace().collect::<Vec<_>>();
        let mut contexts = Vec::new();
        for index in 0..tokens.len().saturating_sub(1) {
            if !matches!((tokens[index], tokens[index + 1]), ("=" | "!", "=")) {
                continue;
            }
            let start = index.saturating_sub(4);
            let end = (index + 6).min(tokens.len());
            if tokens[start..index].contains(&"run") || tokens[index + 2..end].contains(&"run") {
                contexts.push(tokens[start..end].join(" "));
            }
        }
        contexts
    }

    #[test]
    fn observational_service_types_cannot_derive_whole_object_equality() {
        let derived = r"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDescriptor { pub run: Uuid }
";
        let explicit = r"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Earlier;
pub struct ServiceDescriptor { pub run: Uuid }
impl PartialEq for ServiceDescriptor {
    fn eq(&self, _other: &Self) -> bool { true }
}
";

        assert!(struct_derives_trait(
            derived,
            "ServiceDescriptor",
            "PartialEq"
        ));
        assert!(!struct_derives_trait(
            explicit,
            "ServiceDescriptor",
            "PartialEq"
        ));
    }

    #[test]
    fn production_source_reconcile_uses_the_assetdb_writer() {
        let workspace_root = guard_workspace_root();
        for relative_path in [
            "crates/az/asset-processor/src/lib.rs",
            "crates/az/asset-processor/src/watcher.rs",
        ] {
            let source = production_source_without_cfg_test_modules(
                &fs::read_to_string(workspace_root.join(relative_path))
                    .unwrap_or_else(|error| panic!("read {relative_path}: {error}")),
            );
            let compact = source
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            assert!(
                !compact.contains(".reconcile_source_assets(")
                    && !compact.contains(".reconcile_source_assets_with_records("),
                "{relative_path}: production source reconciliation must submit its owned batch through AssetDbWriter"
            );
        }

        let repository_source =
            fs::read_to_string(workspace_root.join("crates/az/assetdb/src/repo.rs"))
                .expect("read assetdb repository source");
        assert!(public_functions_are_cfg_test_or_test_support(
            &repository_source,
            "pub fn reconcile_source_assets("
        ));
        assert!(public_functions_are_cfg_test_or_test_support(
            &repository_source,
            "pub fn reconcile_source_assets_with_records("
        ));
    }

    #[test]
    fn assetdb_mutation_surface_is_owned_by_the_writer_command_inventory() {
        let workspace_root = guard_workspace_root();
        let repository = fs::read_to_string(workspace_root.join("crates/az/assetdb/src/repo.rs"))
            .expect("read assetdb repository source");
        // Production visibility is enforced by Rust: direct fixture mutators
        // are absent unless the test-support feature is selected, while live
        // callers compile only against AssetDbWriter. Keep the public mutation
        // methods paired with the commands consumed by the single writer.
        let commands = repository
            .split("enum WriterCommand {")
            .nth(1)
            .expect("find AssetDB writer command inventory")
            .split("struct WriterInner")
            .next()
            .expect("find end of AssetDB writer command inventory");
        let writer = repository
            .split("impl AssetDbWriter {")
            .nth(1)
            .expect("find AssetDbWriter implementation")
            .split("impl AssetDb {")
            .next()
            .expect("find end of AssetDbWriter implementation");
        assert!(symbols_contain(&repository, "fn writer_loop("));
        assert!(symbols_contain(
            &repository,
            "fn handle_late_writer_command("
        ));
        for (command, method) in [
            ("RegisterWorkspace", "register_workspace"),
            ("RegisterWorkspaceRoot", "register_workspace_root"),
            ("ReplaceWorkspaceRoots", "replace_workspace_roots"),
            ("ApplySweep", "apply_sweep_delta"),
            ("ReplaceBuilders", "replace_builder_catalog"),
            ("ApplyPlan", "apply_plan_delta"),
            ("Claim", "claim_ready_job"),
            ("Abandon", "abandon_attempts"),
            ("Complete", "complete_attempt"),
            ("ResolveIdle", "resolve_idle_blocked"),
            ("ImportPayload", "import_unsaved_payload"),
            ("WritePayload", "write_source_payload"),
            ("PublishAuthoredSource", "publish_authored_source"),
            ("MoveSource", "move_source"),
            ("DeleteSource", "delete_source"),
        ] {
            assert!(
                symbols_contain(commands, command),
                "WriterCommand must own `{command}`"
            );
            assert!(
                symbols_contain(writer, &format!("pub fn {method}(")),
                "AssetDbWriter must expose `{method}`"
            );
        }
    }

    #[test]
    fn public_function_gate_helper_accepts_test_support_cfgs() {
        let source = r#"
#[cfg(any(test, feature = "test-support"))]
pub fn harness_api() {}

#[cfg(test)]
pub fn test_api() {}
"#;

        assert!(public_functions_are_cfg_test_or_test_support(
            source,
            "pub fn harness_api("
        ));
        assert!(public_functions_are_cfg_test_or_test_support(
            source,
            "pub fn test_api("
        ));
        assert!(!public_functions_are_cfg_test_or_test_support(
            "pub fn production_api() {}",
            "pub fn production_api("
        ));
    }
}
