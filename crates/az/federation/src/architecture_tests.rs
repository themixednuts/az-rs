// test-only module
//
// Structural guards for the Certified Federation crates: `az-federation`,
// `az-federation-crypto`, and `az-proto-federation`.
//
// Linear ADR 0053 states these boundaries as prose. Each guard here turns one
// of them into a failing test, and each is paired with a fixture-violation test
// so a green run means "the rule fires and this tree passes it", never "the
// scanner found nothing".
//
// The guards live in the kernel crate and scan all three crate roots, so a
// boundary break in any of them fails `cargo test -p az-federation`.

use std::{collections::BTreeSet, path::Path};

use az_architecture_guard::{
    DependencyBoundary, ProductionSource, collect_production_rust_sources,
    forbidden_production_dependencies, symbols_contain,
};
use toml::Value;

const FEDERATION_MANIFEST: &str = include_str!("../Cargo.toml");
const FEDERATION_CRYPTO_MANIFEST: &str = include_str!("../../federation-crypto/Cargo.toml");
const PROTO_FEDERATION_MANIFEST: &str = include_str!("../../proto/federation/Cargo.toml");
const WORKSPACE_MANIFEST: &str = include_str!("../../../../Cargo.toml");

/// The lifecycle owner's public surface, read as data so this guard keeps
/// tracking ADR 0052 as that crate grows.
const WORLD_INSTANCE_LIB: &str = include_str!("../../world-instance/src/lib.rs");

/// Provider SDKs, runtimes, and databases. ADR 0053 keeps every one of these
/// below an adapter; none may appear in a federation crate's production graph.
const PROVIDER_DEPENDENCIES: &[&str] = &[
    "async-std",
    "aws-config",
    "aws-sdk-secretsmanager",
    "axum",
    "cloudflare",
    "hyper",
    "libsql",
    "mongodb",
    "redis",
    "reqwest",
    "rusqlite",
    "serde_json",
    "smol",
    "sqlx",
    "tonic",
    "tower",
    "wasm-bindgen",
    "worker",
];

/// Public shapes that hide ownership: a generic backend, a service locator, an
/// erased command bus, or an untyped policy bag.
const PROVIDER_SHAPED_SYMBOLS: &[&str] = &[
    "FederationBackend",
    "ServiceLocator",
    "CommandBus",
    "dyn Any",
    "serde_json::Value",
];

/// Name endings that describe a provider or ambient dependency holder rather
/// than a domain concept or a narrow port.
const PROVIDER_SHAPED_SUFFIXES: &[&str] = &[
    "Backend", "Bus", "Facade", "Helper", "Locator", "Manager", "Registry", "Util",
];

/// The lifecycle control surface ADR 0052 reserves for `az-world-instance`.
/// Federation reads one placement fact through its own port and never drives
/// placement, admission, ingress, or transfer.
const LIFECYCLE_CONTROL_SURFACE: &[&str] = &[
    "WorldInstanceService",
    "ApplyWorldInstance",
    "PlacementProvider",
    "IngressControl",
    "ReleaseDistribution",
];

/// Where a federation dependency's crate may live.
///
/// The rule is positive: a production dependency either resolves into the
/// engine tree or is an allowlisted registry crate. Gems, project crates,
/// generated title bindings, and out-of-tree paths then fail by construction,
/// so this guard never has to carry a game's name.
const ENGINE_DEPENDENCY_PATH_ROOTS: &[&str] = &["crates/az/", "crates/integrations/"];

/// Registry crates the federation production graph may link. Adding one is a
/// deliberate review step; every other published crate fails by construction,
/// including database, provider, and durable-substrate clients.
const ALLOWED_REGISTRY_DEPENDENCIES: &[&str] = &[
    "arrayvec",
    "blake3",
    "ed25519-dalek",
    "serde",
    "thiserror",
    "uuid",
];

/// Engine crates that live under an allowed root and still hold storage,
/// secret, or durable-substrate authority. ADR 0051 substrate and ADR 0038
/// secret resolution reach federation through ports and composition roots.
const SUBSTRATE_AUTHORITY_DEPENDENCIES: &[&str] = &[
    "az-assetdb",
    "az-durable",
    "az-durable-file",
    "az-durable-turso",
    "az-secret-value",
    "az-secrets",
    "az-secrets-aws",
    "az-turso",
];

/// Gem and imported legacy-format crate prefixes.
const GAME_LAYER_DEPENDENCY_PREFIXES: &[&str] = &["az-gem-", "cry-"];

/// Direct writer shapes: a game reducer entrypoint or its ambient caller
/// identity. Growth here is exactly what ADR 0053's cutover must shrink.
const DIRECT_WRITER_SYMBOLS: &[&str] = &["reducer", "ctx.sender()", "ReducerContext"];

/// Simulation hosts and executors. A federation crate that cannot link one
/// cannot be scheduled into a simulation frame.
const SIMULATION_HOST_DEPENDENCIES: &[&str] = &[
    "async-executor",
    "az-durable-bevy",
    "az-engine",
    "az-framework",
    "az-runtime-app",
    "az-runtime-host",
    "az-work",
    "bevy",
    "bevy_app",
    "bevy_ecs",
    "tokio",
];

/// Calls that turn an authority wait into a blocking one, and the schedule
/// labels that would place such a wait inside a frame.
const SCHEDULE_BLOCKING_SYMBOLS: &[&str] = &[
    "block_on",
    "spawn_blocking",
    "thread::sleep",
    "add_systems",
    "FixedUpdate",
    "PreUpdate",
    "PostUpdate",
];

/// Trait-name endings that declare an effect boundary. Every method on one
/// must hand back a future so a caller has to own the wait explicitly.
const EFFECT_TRAIT_SUFFIXES: &[&str] = &["Port", "Custody"];

fn workspace_manifest() -> Value {
    toml::from_str(WORKSPACE_MANIFEST).expect("workspace manifest")
}

/// Each federation crate as `(package, workspace-relative directory, manifest)`.
///
/// The directory is what lets a dependency path resolve to a workspace-rooted
/// crate location instead of a manifest-relative string.
fn federation_manifests() -> Vec<(&'static str, &'static str, Value)> {
    [
        ("az-federation", "crates/az/federation", FEDERATION_MANIFEST),
        (
            "az-federation-crypto",
            "crates/az/federation-crypto",
            FEDERATION_CRYPTO_MANIFEST,
        ),
        (
            "az-proto-federation",
            "crates/az/proto/federation",
            PROTO_FEDERATION_MANIFEST,
        ),
    ]
    .into_iter()
    .map(|(name, directory, manifest)| {
        (
            name,
            directory,
            toml::from_str(manifest).expect("crate manifest"),
        )
    })
    .collect()
}

fn federation_sources() -> Vec<ProductionSource> {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        crate_root.join("src"),
        crate_root.join("../federation-crypto/src"),
        crate_root.join("../proto/federation/src"),
    ]
    .into_iter()
    .flat_map(|root| collect_production_rust_sources(root).expect("federation production sources"))
    .collect()
}

/// Every type, trait, and alias a source declares, at any visibility.
///
/// A duplicated lifecycle hidden behind a private type is still a duplicated
/// lifecycle, so visibility is deliberately ignored.
fn declared_item_names(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let line = line.strip_prefix("pub ").unwrap_or(line);
            let line = line
                .split_once(')')
                .filter(|(visibility, _)| visibility.starts_with("(crate"))
                .map_or(line, |(_, rest)| rest.trim_start());
            let rest = ["struct ", "enum ", "trait ", "type "]
                .iter()
                .find_map(|keyword| line.strip_prefix(keyword))?;
            let name: String = rest
                .chars()
                .take_while(|character| character.is_alphanumeric() || *character == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// Type and trait names exported by the lifecycle crate.
fn lifecycle_owned_names(lifecycle_lib: &str) -> BTreeSet<String> {
    let mut owned = BTreeSet::new();
    let mut rest = lifecycle_lib;
    while let Some(start) = rest.find("pub use ") {
        let after = &rest[start + "pub use ".len()..];
        let Some(end) = after.find(';') else { break };
        for token in after[..end].split(['{', '}', ',']) {
            let token = token.trim();
            let name = token.rsplit("::").next().unwrap_or(token).trim();
            if name.starts_with(char::is_uppercase)
                && name
                    .chars()
                    .all(|character| character.is_alphanumeric() || character == '_')
            {
                owned.insert(name.to_owned());
            }
        }
        rest = &after[end..];
    }
    owned
}

fn provider_dependency_boundary() -> DependencyBoundary<'static> {
    DependencyBoundary::new(PROVIDER_DEPENDENCIES, &[], &[])
}

fn simulation_host_boundary() -> DependencyBoundary<'static> {
    DependencyBoundary::new(SIMULATION_HOST_DEPENDENCIES, &["bevy-", "bevy_"], &[])
}

fn forbidden_dependencies(
    manifests: &[(&'static str, &'static str, Value)],
    workspace: &Value,
    boundary: DependencyBoundary<'_>,
) -> Vec<String> {
    manifests
        .iter()
        .flat_map(|(name, _, manifest)| {
            forbidden_production_dependencies(manifest, workspace, boundary)
                .into_iter()
                .map(move |violation| format!("{name}: {violation}"))
        })
        .collect()
}

/// One production dependency resolved through its workspace alias.
struct ResolvedDependency {
    section: String,
    alias: String,
    package: String,
    /// Workspace-root-relative crate directory, when the dependency has one.
    path: Option<String>,
}

impl ResolvedDependency {
    /// The package name plus the crate directory a bare path dependency names.
    fn crate_identities(&self) -> Vec<&str> {
        let mut identities = vec![self.package.as_str()];
        if let Some(directory) = self
            .path
            .as_deref()
            .and_then(|path| path.rsplit('/').next())
        {
            identities.push(directory);
        }
        identities
    }
}

/// Walks `[dependencies]`, `[build-dependencies]`, and their target tables.
///
/// `az-architecture-guard` owns the same traversal for forbidden name and path
/// lists. This guard states its rule positively, so it needs the resolved path
/// itself, and that resolution is private there.
fn production_dependencies(
    crate_directory: &str,
    manifest: &Value,
    workspace: &Value,
) -> Vec<ResolvedDependency> {
    let mut resolved = Vec::new();
    for (section, table) in dependency_tables(manifest) {
        let Some(entries) = table.as_table() else {
            continue;
        };
        for (alias, spec) in entries {
            let (package, path) = resolve_dependency(alias, spec, workspace, crate_directory);
            resolved.push(ResolvedDependency {
                section: section.clone(),
                alias: alias.clone(),
                package,
                path,
            });
        }
    }
    resolved
}

/// Production dependency tables, excluding test-only and workspace tables.
fn dependency_tables(manifest: &Value) -> Vec<(String, &Value)> {
    let mut tables = Vec::new();
    for name in ["dependencies", "build-dependencies"] {
        if let Some(table) = manifest.get(name) {
            tables.push((name.to_owned(), table));
        }
    }
    if let Some(targets) = manifest.get("target").and_then(Value::as_table) {
        for (target, spec) in targets {
            for name in ["dependencies", "build-dependencies"] {
                if let Some(table) = spec.get(name) {
                    tables.push((format!("target.{target}.{name}"), table));
                }
            }
        }
    }
    tables
}

/// Resolves one entry to its package name and workspace-relative path.
///
/// A workspace alias resolves against the workspace root; a directly declared
/// path resolves against the crate's own directory.
fn resolve_dependency(
    alias: &str,
    spec: &Value,
    workspace: &Value,
    crate_directory: &str,
) -> (String, Option<String>) {
    let aliased = spec
        .get("workspace")
        .and_then(Value::as_bool)
        .unwrap_or_default()
        .then(|| {
            workspace
                .get("workspace")
                .and_then(|table| table.get("dependencies"))
                .and_then(|table| table.get(alias))
        })
        .flatten();
    let (entry, base) = aliased.map_or((spec, crate_directory), |entry| (entry, ""));
    let package = entry
        .get("package")
        .and_then(Value::as_str)
        .unwrap_or(alias)
        .to_owned();
    let path = entry
        .get("path")
        .and_then(Value::as_str)
        .map(|path| normalize_path(base, path));
    (package, path)
}

/// Lexically resolves `path` against `base` without touching the filesystem.
fn normalize_path(base: &str, path: &str) -> String {
    let path = path.replace('\\', "/");
    let mut segments: Vec<&str> = Vec::new();
    for segment in base.split('/').chain(path.split('/')) {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            named => segments.push(named),
        }
    }
    segments.join("/")
}

/// Dependencies that leave the engine tree, are unallowlisted registry crates,
/// or carry storage, secret, gem, or legacy-format authority.
fn storage_authority_violations(
    manifests: &[(&'static str, &'static str, Value)],
    workspace: &Value,
) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, directory, manifest) in manifests {
        for dependency in production_dependencies(directory, manifest, workspace) {
            let location = format!("{name}: {}.{}", dependency.section, dependency.alias);
            match &dependency.path {
                Some(path) => {
                    if !ENGINE_DEPENDENCY_PATH_ROOTS
                        .iter()
                        .any(|root| path.starts_with(root))
                    {
                        violations.push(format!(
                            "{location} resolves outside the engine tree ({path})"
                        ));
                    }
                }
                None => {
                    if !ALLOWED_REGISTRY_DEPENDENCIES.contains(&dependency.package.as_str()) {
                        violations.push(format!(
                            "{location} is not an allowlisted registry crate ({})",
                            dependency.package
                        ));
                    }
                }
            }
            for identity in dependency.crate_identities() {
                if SUBSTRATE_AUTHORITY_DEPENDENCIES.contains(&identity) {
                    violations.push(format!(
                        "{location} carries storage or secret authority ({identity})"
                    ));
                }
                if GAME_LAYER_DEPENDENCY_PREFIXES
                    .iter()
                    .any(|prefix| identity.starts_with(prefix))
                {
                    violations.push(format!(
                        "{location} is a gem or legacy-format crate ({identity})"
                    ));
                }
            }
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

fn symbol_violations(sources: &[ProductionSource], symbols: &[&str]) -> Vec<String> {
    sources
        .iter()
        .flat_map(|source| {
            symbols
                .iter()
                .filter(|symbol| symbols_contain(&source.source, symbol))
                .map(|symbol| format!("{}: {symbol}", source.path.display()))
        })
        .collect()
}

fn provider_shaped_name_violations(sources: &[ProductionSource]) -> Vec<String> {
    sources
        .iter()
        .flat_map(|source| {
            declared_item_names(&source.source)
                .into_iter()
                .filter(|name| {
                    PROVIDER_SHAPED_SUFFIXES
                        .iter()
                        .any(|suffix| name.ends_with(suffix))
                })
                .map(|name| format!("{}: {name}", source.path.display()))
        })
        .collect()
}

fn duplicate_lifecycle_violations(
    sources: &[ProductionSource],
    owned: &BTreeSet<String>,
) -> Vec<String> {
    sources
        .iter()
        .flat_map(|source| {
            declared_item_names(&source.source)
                .into_iter()
                .filter(|name| owned.contains(name))
                .map(|name| format!("{}: {name}", source.path.display()))
        })
        .collect()
}

/// Effect-boundary trait methods that return anything other than a future.
///
/// The scan walks each `trait Name…Port`/`…Custody` body and checks every `fn`
/// signature up to its terminating `;` or `{`.
fn synchronous_effect_port_methods(sources: &[ProductionSource]) -> Vec<String> {
    let mut violations = Vec::new();
    for source in sources {
        for (name, body) in effect_trait_bodies(&source.source) {
            for method in body.split("fn ").skip(1) {
                let signature = method
                    .split_once(';')
                    .map_or(method, |(signature, _)| signature);
                if !signature.contains("Future") {
                    let method_name: String = signature
                        .chars()
                        .take_while(|character| character.is_alphanumeric() || *character == '_')
                        .collect();
                    violations.push(format!("{}: {name}::{method_name}", source.path.display()));
                }
            }
        }
    }
    violations
}

/// Returns `(trait name, brace-balanced body)` for each effect-boundary trait.
fn effect_trait_bodies(source: &str) -> Vec<(String, String)> {
    let mut bodies = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find("trait ") {
        let after = &rest[start + "trait ".len()..];
        let name: String = after
            .chars()
            .take_while(|character| character.is_alphanumeric() || *character == '_')
            .collect();
        let Some(open) = after.find('{') else { break };
        let mut depth = 0i32;
        let mut end = open;
        for (index, character) in after[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + index;
                        break;
                    }
                }
                _ => {}
            }
        }
        if EFFECT_TRAIT_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
        {
            bodies.push((name, after[open..=end].to_owned()));
        }
        rest = &after[open..];
    }
    bodies
}

fn fixture(path: &str, source: &str) -> ProductionSource {
    ProductionSource {
        path: Path::new(path).to_path_buf(),
        source: source.to_owned(),
    }
}

#[test]
fn federation_crates_expose_no_provider_shaped_public_api() {
    let workspace = workspace_manifest();
    let dependency_violations = forbidden_dependencies(
        &federation_manifests(),
        &workspace,
        provider_dependency_boundary(),
    );
    let sources = federation_sources();
    let symbol_violations = symbol_violations(&sources, PROVIDER_SHAPED_SYMBOLS);
    let name_violations = provider_shaped_name_violations(&sources);

    assert!(
        dependency_violations.is_empty(),
        "provider SDKs, runtimes, and databases belong below an adapter: {dependency_violations:?}"
    );
    assert!(
        symbol_violations.is_empty(),
        "federation exposes no generic backend, locator, command bus, or untyped policy bag: {symbol_violations:?}"
    );
    assert!(
        name_violations.is_empty(),
        "federation types name domain concepts and narrow ports, not dependency holders: {name_violations:?}"
    );
}

#[test]
fn provider_guard_fires_on_a_backend_dependency_and_a_locator_type() {
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        store = { package = "sqlx", version = "0.8" }

        [target.'cfg(windows)'.dependencies]
        edge = { workspace = true }
        "#,
    )
    .expect("fixture manifest");
    let workspace: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        edge = { package = "worker", version = "0.4" }
        "#,
    )
    .expect("fixture workspace manifest");

    assert_eq!(
        forbidden_dependencies(
            &[("fixture", "crates/az/fixture", manifest)],
            &workspace,
            provider_dependency_boundary()
        ),
        vec![
            "fixture: dependencies.store(package sqlx)".to_owned(),
            "fixture: target.cfg(windows).dependencies.edge(workspace package worker)".to_owned(),
        ]
    );

    let sources = vec![fixture(
        "fixture/locator.rs",
        "pub struct FederationServiceLocator;\n\
         pub trait FederationBackend {}\n",
    )];
    assert_eq!(
        provider_shaped_name_violations(&sources),
        vec![
            "fixture/locator.rs: FederationServiceLocator".to_owned(),
            "fixture/locator.rs: FederationBackend".to_owned(),
        ]
    );
    assert_eq!(
        symbol_violations(&sources, PROVIDER_SHAPED_SYMBOLS),
        vec!["fixture/locator.rs: FederationBackend".to_owned()]
    );
}

#[test]
fn federation_declares_no_second_placement_admission_or_transfer_lifecycle() {
    let owned = lifecycle_owned_names(WORLD_INSTANCE_LIB);
    assert!(
        owned.contains("PlacementFence"),
        "the lifecycle export scan must find ADR 0052's fence type"
    );

    let sources = federation_sources();
    let duplicates = duplicate_lifecycle_violations(&sources, &owned);
    let control_surface = symbol_violations(&sources, LIFECYCLE_CONTROL_SURFACE);

    assert!(
        duplicates.is_empty(),
        "placement, admission, presence, and transfer types stay owned by az-world-instance: {duplicates:?}"
    );
    assert!(
        control_surface.is_empty(),
        "federation reads one placement fact through its own port and drives no lifecycle: {control_surface:?}"
    );
}

#[test]
fn lifecycle_guard_fires_on_a_duplicate_ticket_type_and_a_lifecycle_call() {
    let owned = lifecycle_owned_names(
        "pub use identity::{AdmissionTicketId, PlacementFence};\npub use lifecycle::WorldInstanceService;\n",
    );
    assert!(owned.contains("AdmissionTicketId") && owned.contains("WorldInstanceService"));

    let sources = vec![fixture(
        "fixture/lifecycle.rs",
        "pub struct AdmissionTicketId(u64);\n\
         async fn admit(service: &WorldInstanceService) {\n\
             let _ = service.apply(request).await;\n\
         }\n",
    )];

    assert_eq!(
        duplicate_lifecycle_violations(&sources, &owned),
        vec!["fixture/lifecycle.rs: AdmissionTicketId".to_owned()]
    );
    assert_eq!(
        symbol_violations(&sources, LIFECYCLE_CONTROL_SURFACE),
        vec!["fixture/lifecycle.rs: WorldInstanceService".to_owned()]
    );
}

#[test]
fn federation_holds_no_direct_game_auth_or_storage_authority() {
    let workspace = workspace_manifest();
    let manifests = federation_manifests();
    let dependency_violations = storage_authority_violations(&manifests, &workspace);
    let writer_violations = symbol_violations(&federation_sources(), DIRECT_WRITER_SYMBOLS);

    assert!(
        dependency_violations.is_empty(),
        "every federation dependency resolves into the engine tree or an allowlisted registry crate, and none carries storage, secret, gem, or legacy-format authority: {dependency_violations:?}"
    );
    assert!(
        writer_violations.is_empty(),
        "federation writes through its authority atom, never a direct game reducer: {writer_violations:?}"
    );
}

#[test]
fn the_dependency_scan_resolves_workspace_aliases_to_engine_paths() {
    let workspace = workspace_manifest();
    let resolved = production_dependencies(
        "crates/az/federation",
        &toml::from_str::<Value>(FEDERATION_MANIFEST).expect("crate manifest"),
        &workspace,
    );
    let resolved = resolved
        .iter()
        .map(|dependency| (dependency.package.as_str(), dependency.path.as_deref()))
        .collect::<BTreeSet<_>>();

    assert!(
        resolved.contains(&("az-world-instance", Some("crates/az/world-instance"))),
        "a workspace alias must resolve to its workspace-rooted crate path; scan found {resolved:?}"
    );
    assert!(
        resolved.contains(&("uuid", None)),
        "a registry dependency must resolve with no path; scan found {resolved:?}"
    );
}

#[test]
fn storage_authority_guard_fires_on_a_direct_reducer_and_an_out_of_tree_dependency() {
    // `crypto` proves a legitimate workspace alias resolves and passes; the
    // remaining entries each trip one clause of the positive rule.
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        bindings = { package = "sample-title-bindings", path = "../../../sample-title/crates/bindings" }
        crypto = { workspace = true }
        durable = { package = "az-durable", path = "../durable" }
        gem = { path = "../../gems/az-gem-auth" }
        vendor = "1.0"

        [target.'cfg(windows)'.dependencies]
        store = { workspace = true }
        "#,
    )
    .expect("fixture manifest");
    let workspace: Value = toml::from_str(
        r#"
        [workspace.dependencies]
        crypto = { package = "az-federation-crypto", path = "crates/az/federation-crypto" }
        store = { package = "vendor-db-client", version = "1" }
        "#,
    )
    .expect("fixture workspace manifest");

    assert_eq!(
        storage_authority_violations(&[("fixture", "crates/az/fixture", manifest)], &workspace),
        vec![
            "fixture: dependencies.bindings resolves outside the engine tree (sample-title/crates/bindings)".to_owned(),
            "fixture: dependencies.durable carries storage or secret authority (az-durable)".to_owned(),
            "fixture: dependencies.gem is a gem or legacy-format crate (az-gem-auth)".to_owned(),
            "fixture: dependencies.gem resolves outside the engine tree (crates/gems/az-gem-auth)".to_owned(),
            "fixture: dependencies.vendor is not an allowlisted registry crate (vendor)".to_owned(),
            "fixture: target.cfg(windows).dependencies.store is not an allowlisted registry crate (vendor-db-client)".to_owned(),
        ]
    );

    let sources = vec![fixture(
        "fixture/writer.rs",
        "fn apply(ctx: &ReducerContext) {\n\
             let owner = ctx.sender();\n\
         }\n",
    )];
    assert_eq!(
        symbol_violations(&sources, DIRECT_WRITER_SYMBOLS),
        vec![
            "fixture/writer.rs: ctx.sender()".to_owned(),
            "fixture/writer.rs: ReducerContext".to_owned(),
        ]
    );
}

#[test]
fn federation_authority_waits_cannot_run_inside_a_simulation_schedule() {
    let workspace = workspace_manifest();
    let dependency_violations = forbidden_dependencies(
        &federation_manifests(),
        &workspace,
        simulation_host_boundary(),
    );
    let sources = federation_sources();
    let blocking_violations = symbol_violations(&sources, SCHEDULE_BLOCKING_SYMBOLS);
    let synchronous_ports = synchronous_effect_port_methods(&sources);

    assert!(
        dependency_violations.is_empty(),
        "a federation crate that cannot link a simulation host cannot be scheduled into a frame: {dependency_violations:?}"
    );
    assert!(
        blocking_violations.is_empty(),
        "authority work is never blocked on or attached to a frame schedule: {blocking_violations:?}"
    );
    assert!(
        synchronous_ports.is_empty(),
        "every effect-boundary method returns a future so the caller owns the wait: {synchronous_ports:?}"
    );
}

#[test]
fn hot_path_guard_fires_on_a_blocking_call_and_a_synchronous_effect_port() {
    let manifest: Value = toml::from_str(
        r#"
        [dependencies]
        engine = { package = "bevy", version = "0.18" }
        "#,
    )
    .expect("fixture manifest");
    let workspace: Value = toml::from_str("[workspace.dependencies]\n").expect("fixture workspace");

    assert_eq!(
        forbidden_dependencies(
            &[("fixture", "crates/az/fixture", manifest)],
            &workspace,
            simulation_host_boundary()
        ),
        vec!["fixture: dependencies.engine(package bevy)".to_owned()]
    );

    let sources = vec![fixture(
        "fixture/schedule.rs",
        "fn tick(app: &mut App) {\n\
             app.add_systems(FixedUpdate, settle);\n\
             let _ = futures::executor::block_on(commit());\n\
         }\n\
         pub trait AuthorityAtomPort {\n\
             fn commit(&self) -> Result<(), StorageFailure>;\n\
             fn observe(&self) -> AuthorityAtomFuture<'_>;\n\
         }\n",
    )];

    assert_eq!(
        symbol_violations(&sources, SCHEDULE_BLOCKING_SYMBOLS),
        vec![
            "fixture/schedule.rs: block_on".to_owned(),
            "fixture/schedule.rs: add_systems".to_owned(),
            "fixture/schedule.rs: FixedUpdate".to_owned(),
        ]
    );
    assert_eq!(
        synchronous_effect_port_methods(&sources),
        vec!["fixture/schedule.rs: AuthorityAtomPort::commit".to_owned()]
    );
}

#[test]
fn every_guard_reads_all_three_federation_crate_roots() {
    let sources = federation_sources();
    for crate_root in ["federation", "federation-crypto", "proto"] {
        assert!(
            sources.iter().any(|source| {
                source
                    .path
                    .components()
                    .any(|component| component.as_os_str() == crate_root)
            }),
            "the source scan must reach crates/az/{crate_root}; a silent path change would make every source guard vacuous"
        );
    }
    assert!(
        sources
            .iter()
            .any(|source| source.source.contains("trait ")),
        "the scan must return source text, not empty files"
    );
}

#[test]
fn the_effect_port_scan_reads_every_federation_port() {
    let scanned = federation_sources()
        .iter()
        .flat_map(|source| effect_trait_bodies(&source.source))
        .map(|(name, _)| name)
        .collect::<BTreeSet<_>>();

    for expected in [
        "AuthorityAtomPort",
        "AuthorityOperationLookupPort",
        "PlacementAuthorityPort",
        "TransparencyOutboxPort",
        "ChallengeVerificationPort",
        "SigningPort",
        "SigningKeyCustody",
        "VerificationPort",
    ] {
        assert!(
            scanned.contains(expected),
            "the effect-boundary scan must reach {expected}; it found {scanned:?}"
        );
    }
}
