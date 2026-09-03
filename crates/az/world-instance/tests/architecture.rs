//! Structural guards for the world-instance lifecycle core.
//!
//! Wave 0 of the Dynamic `WorldInstance` migration makes ADR 0052's ownership
//! rules executable before any runtime path exists. Each guard is a pure
//! function over this crate's manifest and sources, so it is proven twice: once
//! against the real crate, where it must find nothing, and once against a
//! fixture that deliberately violates it, where it must fire. A guard that has
//! never fired proves nothing.
//!
//! Comment and documentation lines are excluded from source scans so prose may
//! name the seams it describes. Trailing comments on a code line are still
//! scanned, which fails closed rather than open.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use az_architecture_guard::{
    DependencyBoundary, PROJECT_FORMAT_EXAMPLE_OR_WORKSPACE_PROJECT_PATH_SEGMENTS,
    forbidden_production_dependencies, non_allowlisted_production_dependencies,
};
use toml::Value;

/// Rust sources keyed by their path relative to `src/`.
type Sources = BTreeMap<String, String>;

const MANIFEST: &str = include_str!("../Cargo.toml");
const WORKSPACE_MANIFEST: &str = include_str!("../../../../Cargo.toml");

// ---------------------------------------------------------------------------
// Guard 1: lifecycle core admits only allowlisted namespaces
// ---------------------------------------------------------------------------

/// Production dependencies the lifecycle core is allowed to have.
///
/// This is an allowlist, not a list of things to keep out. A crate nobody
/// thought to forbid still cannot enter the core, and widening the boundary has
/// to be a deliberate edit here. Adapters depend on this crate; this crate
/// depends on generic value crates only.
const ALLOWED_PRODUCTION_DEPENDENCIES: &[&str] = &["serde", "thiserror", "uuid"];

/// Source trees that are outside the engine boundary entirely.
const OUT_OF_BOUNDS_SOURCE_TREES: DependencyBoundary<'_> = DependencyBoundary {
    exact_names: &[],
    name_prefixes: &[],
    path_segments: PROJECT_FORMAT_EXAMPLE_OR_WORKSPACE_PROJECT_PATH_SEGMENTS,
    exempt_names: &[],
    exempt_paths: &[],
};

/// Engine crates that would reverse the lifecycle's dependency direction.
///
/// The allowlists already exclude everything unnamed. This list exists because
/// engine crates are admitted as a *namespace*, and these are the ones that
/// must never be admitted through it: ADR 0052 points the dependency from
/// federation, host, and provider implementations toward this crate, and ADR
/// 0051 supplies durable substrate to adapters rather than to lifecycle core.
const REJECTED_ENGINE_CRATES: &[&str] = &[
    "az-durable",
    "az-durable-bevy",
    "az-durable-file",
    "az-durable-turso",
    "az-federation",
    "az-federation-crypto",
    "az-filesystem",
    "az-rpc",
    "az-secrets",
    "az-session",
    "az-turso",
    "az-world-instance-federation",
    "az-world-instance-host",
];

/// Engine crate families that are adapters or wire records by construction.
const REJECTED_ENGINE_PREFIXES: &[&str] = &["az-gem-", "az-proto-"];

const REJECTED_ENGINE_BOUNDARY: DependencyBoundary<'_> = DependencyBoundary {
    exact_names: REJECTED_ENGINE_CRATES,
    name_prefixes: REJECTED_ENGINE_PREFIXES,
    path_segments: &[],
    exempt_names: &[],
    exempt_paths: &[],
};

/// Namespaces a `use` path may resolve to, beside this crate's own modules.
///
/// Engine crates are admitted by their `az_` namespace and then filtered by
/// [`REJECTED_ENGINE_CRATES`], because lifecycle core may consume generic
/// engine value crates but never an adapter.
const ALLOWED_IMPORT_ROOTS: &[&str] = &[
    "alloc",
    "core",
    "crate",
    "self",
    "serde",
    "std",
    "super",
    "thiserror",
    "uuid",
];

/// Type-name stems that describe a provider, federation, transport, database,
/// or durable-substrate model rather than a lifecycle-owned fact.
const FOREIGN_MODEL_TYPE_STEMS: &[&str] = &[
    "AuthorityAtom",
    "Carrier",
    "Certified",
    "Cloudflare",
    "Dtls",
    "DurableSubject",
    "Federation",
    "GameLift",
    "GameMode",
    "Kubernetes",
    "PortableMutationGrant",
    "Postgres",
    "SpacetimeDb",
    "Turso",
    "WriterFence",
];

fn foreign_model_types(sources: &Sources) -> Vec<String> {
    scan_sources(sources, FOREIGN_MODEL_TYPE_STEMS, "names foreign model")
}

/// Returns the crate or module a `use` path resolves to, if the line is one.
fn import_root(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let path = trimmed
        .strip_prefix("use ")
        .or_else(|| trimmed.strip_prefix("pub use "))
        .or_else(|| trimmed.strip_prefix("pub(crate) use "))?;
    let root = path
        .trim_start()
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .next()?;
    (!root.is_empty()).then(|| root.to_owned())
}

fn is_rejected_engine_crate(import_root: &str) -> bool {
    let name = import_root.replace('_', "-");
    REJECTED_ENGINE_CRATES.contains(&name.as_str())
        || REJECTED_ENGINE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

fn unlisted_import_namespaces(sources: &Sources) -> Vec<String> {
    let mut allowed = ALLOWED_IMPORT_ROOTS
        .iter()
        .map(|root| (*root).to_owned())
        .collect::<BTreeSet<_>>();
    // A crate's own modules are reachable by name from its root.
    allowed.extend(sources.keys().filter_map(|module| {
        module
            .rsplit('/')
            .next()
            .and_then(|file| file.strip_suffix(".rs"))
            .map(ToOwned::to_owned)
    }));

    let mut violations = Vec::new();
    for (module, source) in sources {
        for (line, text) in code_lines(source) {
            let Some(root) = import_root(text) else {
                continue;
            };
            if allowed.contains(&root) {
                continue;
            }
            if root.starts_with("az_") && !is_rejected_engine_crate(&root) {
                continue;
            }
            violations.push(format!(
                "{module}:{line} imports unlisted namespace `{root}`"
            ));
        }
    }
    violations
}

#[test]
fn lifecycle_core_depends_only_on_allowlisted_value_crates() {
    let manifest = toml::from_str::<Value>(MANIFEST).expect("crate manifest");
    let workspace = toml::from_str::<Value>(WORKSPACE_MANIFEST).expect("workspace manifest");

    let unlisted = non_allowlisted_production_dependencies(
        &manifest,
        &workspace,
        ALLOWED_PRODUCTION_DEPENDENCIES,
        OUT_OF_BOUNDS_SOURCE_TREES,
    );
    assert!(
        unlisted.is_empty(),
        "lifecycle core takes generic value crates only; anything else joins \
         ALLOWED_PRODUCTION_DEPENDENCIES deliberately: {}",
        unlisted.join(", ")
    );

    let reversed =
        forbidden_production_dependencies(&manifest, &workspace, REJECTED_ENGINE_BOUNDARY);
    assert!(
        reversed.is_empty(),
        "adapters depend on az-world-instance, not the reverse: {}",
        reversed.join(", ")
    );
}

#[test]
fn lifecycle_core_imports_only_allowlisted_namespaces() {
    let sources = crate_sources();

    // An import reader that found nothing would let this guard pass for the
    // wrong reason, so prove it still resolves the namespaces it admits.
    let roots = sources
        .values()
        .flat_map(|source| code_lines(source).filter_map(|(_, text)| import_root(text)))
        .collect::<BTreeSet<_>>();
    for expected in ["crate", "serde", "std", "uuid"] {
        assert!(
            roots.contains(expected),
            "the import scan must still resolve `{expected}`"
        );
    }

    let violations = unlisted_import_namespaces(&sources);
    assert!(
        violations.is_empty(),
        "every use path must resolve to this crate, the standard library, an admitted engine \
         namespace, or an allowlisted value crate: {}",
        violations.join("; ")
    );
}

#[test]
fn lifecycle_core_declares_no_provider_or_federation_type() {
    let violations = foreign_model_types(&crate_sources());

    assert!(
        violations.is_empty(),
        "lifecycle core must express provider and federation facts through its own \
         consumer-owned ports: {}",
        violations.join("; ")
    );
}

#[test]
fn the_dependency_allowlist_rejects_unlisted_and_reversed_dependencies() {
    let manifest = toml::from_str::<Value>(
        r#"
        [dependencies]
        az-federation = { path = "../federation" }
        trust = { package = "az-world-instance-federation", path = "../world-instance-federation" }
        provider-x = "1"
        sample-title = { path = "../../../projects/sample-title" }
        serde = { workspace = true }
        "#,
    )
    .expect("fixture manifest");
    let workspace =
        toml::from_str::<Value>("[workspace.dependencies]\nserde = \"1\"\n").expect("fixture");

    let unlisted = non_allowlisted_production_dependencies(
        &manifest,
        &workspace,
        ALLOWED_PRODUCTION_DEPENDENCIES,
        OUT_OF_BOUNDS_SOURCE_TREES,
    );
    for rejected in ["az-federation", "provider-x", "sample-title", "trust"] {
        assert!(
            unlisted.iter().any(|entry| entry.contains(rejected)),
            "the allowlist must reject `{rejected}`: {unlisted:?}"
        );
    }
    assert!(
        !unlisted.iter().any(|entry| entry.contains("serde")),
        "an allowlisted value crate must still pass: {unlisted:?}"
    );

    assert_eq!(
        forbidden_production_dependencies(&manifest, &workspace, REJECTED_ENGINE_BOUNDARY),
        vec![
            "dependencies.az-federation",
            "dependencies.trust(package az-world-instance-federation)",
        ]
    );
}

#[test]
fn the_import_allowlist_rejects_project_provider_and_reversed_namespaces() {
    let sources = fixture_sources(&[(
        "placement.rs",
        "use std::fmt;\n\
         use crate::PlacementFence;\n\
         use az_core::AssetId;\n\
         use provider_x::Allocation;\n\
         use sample_title::LaunchRow;\n\
         use az_federation::HostBinding;\n\
         use az_proto_federation::Envelope;\n",
    )]);

    assert_eq!(
        unlisted_import_namespaces(&sources),
        vec![
            "placement.rs:4 imports unlisted namespace `provider_x`".to_owned(),
            "placement.rs:5 imports unlisted namespace `sample_title`".to_owned(),
            "placement.rs:6 imports unlisted namespace `az_federation`".to_owned(),
            "placement.rs:7 imports unlisted namespace `az_proto_federation`".to_owned(),
        ]
    );
}

#[test]
fn the_foreign_model_guard_rejects_a_federation_type_declaration() {
    let sources = fixture_sources(&[(
        "admission.rs",
        "pub struct CertifiedExecutionBinding;\npub fn bind(_: DurableSubject) {}\n",
    )]);

    assert_eq!(
        foreign_model_types(&sources),
        vec![
            "admission.rs:1 names foreign model `Certified`".to_owned(),
            "admission.rs:2 names foreign model `DurableSubject`".to_owned(),
        ]
    );
}

// ---------------------------------------------------------------------------
// Guard 2: one admission, presence, and transfer model
// ---------------------------------------------------------------------------

/// Word stems that belong to the single membership-movement model ADR 0052
/// owns: one admission ticket, one player-presence authority, one transfer
/// saga.
const MEMBERSHIP_MODEL_STEMS: &[&str] =
    &["Admission", "Membership", "Presence", "Ticket", "Transfer"];

/// The only membership-model types the lifecycle core declares today.
///
/// Wave 5 and Wave 7 extend this list as the ticket, presence, and saga records
/// land. A name that appears without joining this list is a second model.
const SANCTIONED_MEMBERSHIP_TYPES: &[&str] =
    &["AdmissionTicketId", "PlayerPresenceId", "WorldTransferId"];

fn duplicate_membership_models(sources: &Sources) -> Vec<String> {
    let sanctioned = SANCTIONED_MEMBERSHIP_TYPES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    let mut violations = Vec::new();
    for (module, source) in sources {
        for (line, name) in declared_type_names(source) {
            if sanctioned.contains(name.as_str()) {
                continue;
            }
            if MEMBERSHIP_MODEL_STEMS
                .iter()
                .any(|stem| name.contains(stem))
            {
                violations.push(format!(
                    "{module}:{line} declares a second membership model `{name}`"
                ));
            }
        }
    }
    violations
}

#[test]
fn lifecycle_core_declares_one_admission_presence_and_transfer_model() {
    let sources = crate_sources();
    let declared = sources
        .values()
        .flat_map(|source| declared_type_names(source))
        .map(|(_, name)| name)
        .collect::<BTreeSet<_>>();

    // An empty scan would let this guard pass for the wrong reason, so prove
    // the declaration reader still finds the sanctioned model first.
    for sanctioned in SANCTIONED_MEMBERSHIP_TYPES {
        assert!(
            declared.contains(*sanctioned),
            "the declaration scan must still find `{sanctioned}`"
        );
    }

    let violations = duplicate_membership_models(&sources);

    assert!(
        violations.is_empty(),
        "there is one admission ticket, one player-presence authority, and one WorldTransfer \
         saga: {}",
        violations.join("; ")
    );
}

#[test]
fn the_membership_model_guard_rejects_a_second_ticket_and_transfer() {
    let sources = fixture_sources(&[
        (
            "admission.rs",
            "pub struct SecondAdmissionTicket;\npub struct AdmissionTicketId;\n",
        ),
        ("transfer.rs", "pub enum TransferState { Started }\n"),
        (
            "identity.rs",
            "refined_identities! {\n    AdmissionTicketId => AdmissionTicket, \"admission ticket\";\n}\n",
        ),
    ]);

    assert_eq!(
        duplicate_membership_models(&sources),
        vec![
            "admission.rs:1 declares a second membership model `SecondAdmissionTicket`".to_owned(),
            "transfer.rs:1 declares a second membership model `TransferState`".to_owned(),
        ]
    );
}

// ---------------------------------------------------------------------------
// Guard 3: no dynamic Cargo target
// ---------------------------------------------------------------------------

/// Identifiers that would let a world instance, game mode, or map produce a
/// build target instead of a runtime record.
const TARGET_GENERATION_SYMBOLS: &[&str] = &[
    "Cargo.toml",
    "Command::new",
    "cargo_metadata",
    "crate-type",
    "std::process",
];

fn dynamic_cargo_target_violations(manifest: &Value, sources: &Sources) -> Vec<String> {
    let mut violations = Vec::new();

    for table in ["bin", "example", "bench", "test"] {
        if manifest.get(table).is_some() {
            violations.push(format!("Cargo.toml declares a `[[{table}]]` target"));
        }
    }

    if manifest
        .get("package")
        .and_then(|package| package.get("build"))
        .is_some()
    {
        violations.push("Cargo.toml declares a build script".to_owned());
    }

    if let Some(crate_type) = manifest
        .get("lib")
        .and_then(|library| library.get("crate-type"))
    {
        let linkage = crate_type
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        if linkage != "rlib" {
            violations.push(format!("Cargo.toml declares crate-type `{linkage}`"));
        }
    }

    violations.extend(scan_sources(
        sources,
        TARGET_GENERATION_SYMBOLS,
        "reaches the build system through",
    ));
    violations
}

#[test]
fn a_world_instance_never_becomes_a_cargo_target() {
    let manifest = toml::from_str::<Value>(MANIFEST).expect("crate manifest");
    let violations = dynamic_cargo_target_violations(&manifest, &crate_sources());

    assert!(
        violations.is_empty(),
        "dynamic instances are runtime records and child processes, never generated Cargo \
         targets: {}",
        violations.join("; ")
    );
}

#[test]
fn the_cargo_target_guard_rejects_generated_targets_and_build_invocation() {
    let manifest = toml::from_str::<Value>(
        r#"
        [package]
        name = "az-world-instance"
        build = "build.rs"

        [lib]
        crate-type = ["cdylib"]

        [[bin]]
        name = "dungeon-instance"
        "#,
    )
    .expect("fixture manifest");
    let sources = fixture_sources(&[(
        "placement.rs",
        "pub fn place() {\n    let _ = std::process::Command::new(\"cargo\");\n}\n",
    )]);

    assert_eq!(
        dynamic_cargo_target_violations(&manifest, &sources),
        vec![
            "Cargo.toml declares a `[[bin]]` target".to_owned(),
            "Cargo.toml declares a build script".to_owned(),
            "Cargo.toml declares crate-type `cdylib`".to_owned(),
            "placement.rs:2 reaches the build system through `Command::new`".to_owned(),
            "placement.rs:2 reaches the build system through `std::process`".to_owned(),
        ]
    );
}

// ---------------------------------------------------------------------------
// Guard 4: no remote effect reachable from a fixed-tick schedule
// ---------------------------------------------------------------------------

/// The only modules that may declare a remote effect.
///
/// Each is a consumer-owned effect port or the convention it follows. A remote
/// effect anywhere else has escaped the control plane.
const REMOTE_EFFECT_MODULES: &[&str] = &[
    "ingress.rs",
    "port.rs",
    "provider.rs",
    "release.rs",
    "store.rs",
];

/// Identifiers that would place code on a simulation schedule.
///
/// Movement, combat, physics, fixed update, and replication are placement-local
/// and never await a provider, store, or remote authority, so lifecycle core
/// names no schedule at all.
const FIXED_TICK_SYMBOLS: &[&str] = &[
    "FixedMain",
    "FixedPostUpdate",
    "FixedPreUpdate",
    "FixedUpdate",
    "IntoSystemConfigs",
    "Replication",
    "Schedule",
    "SystemSet",
    "add_systems",
    "fixed_tick",
    "fixed_update",
    "replicate",
];

fn remote_effects_reachable_from_fixed_tick(sources: &Sources) -> Vec<String> {
    let sanctioned = REMOTE_EFFECT_MODULES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let mut violations = scan_sources(sources, FIXED_TICK_SYMBOLS, "runs on a simulation schedule");

    for (module, source) in sources {
        if sanctioned.contains(module.as_str()) {
            continue;
        }
        for (line, text) in code_lines(source) {
            if text.contains("PortFuture<") || text.contains("PortStream<") {
                violations.push(format!(
                    "{module}:{line} declares a remote effect outside the effect ports"
                ));
            }
        }
    }
    violations.sort();
    violations
}

#[test]
fn no_remote_effect_is_reachable_from_a_fixed_tick_schedule() {
    let sources = crate_sources();

    // The guard is only meaningful while remote effects exist to be misplaced,
    // so prove the scan still sees them in the modules that may declare them.
    let effect_modules = sources
        .iter()
        .filter(|(_, source)| code_lines(source).any(|(_, text)| text.contains("PortFuture<")))
        .map(|(module, _)| module.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        effect_modules,
        REMOTE_EFFECT_MODULES
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        "every effect-port module must still declare a remote effect"
    );

    let violations = remote_effects_reachable_from_fixed_tick(&sources);

    assert!(
        violations.is_empty(),
        "remote effects belong to control-plane services and durable reconcilers, never to a \
         simulation schedule: {}",
        violations.join("; ")
    );
}

#[test]
fn the_fixed_tick_guard_rejects_a_scheduled_remote_effect() {
    let sources = fixture_sources(&[(
        "simulation.rs",
        "pub fn register(app: &mut App) {\n\
         \x20   app.add_systems(FixedUpdate, publish_route);\n\
         }\n\
         fn publish_route(control: &dyn IngressControl) -> PortFuture<'_, ()> {\n\
         \x20   control.publish()\n\
         }\n",
    )]);

    assert_eq!(
        remote_effects_reachable_from_fixed_tick(&sources),
        vec![
            "simulation.rs:2 runs on a simulation schedule `FixedUpdate`".to_owned(),
            "simulation.rs:2 runs on a simulation schedule `add_systems`".to_owned(),
            "simulation.rs:4 declares a remote effect outside the effect ports".to_owned(),
        ]
    );
}

// ---------------------------------------------------------------------------
// Shared scanning
// ---------------------------------------------------------------------------

fn crate_sources() -> Sources {
    let mut sources = Sources::new();
    read_sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        "",
        &mut sources,
    );
    assert!(
        sources.contains_key("lib.rs"),
        "the source scan must reach the crate root"
    );
    sources
}

fn read_sources(directory: &Path, prefix: &str, sources: &mut Sources) {
    for entry in fs::read_dir(directory).expect("readable source directory") {
        let entry = entry.expect("readable directory entry");
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            read_sources(&path, &format!("{prefix}{name}/"), sources);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.insert(
                format!("{prefix}{name}"),
                fs::read_to_string(&path).expect("readable source file"),
            );
        }
    }
}

fn fixture_sources(files: &[(&str, &str)]) -> Sources {
    files
        .iter()
        .map(|(name, source)| ((*name).to_owned(), (*source).to_owned()))
        .collect()
}

/// Yields one-based line numbers and text for lines that are not comments.
fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !(trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*"))
        })
}

fn scan_sources(sources: &Sources, symbols: &[&str], reason: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for (module, source) in sources {
        for (line, text) in code_lines(source) {
            for symbol in symbols {
                if text.contains(symbol) {
                    violations.push(format!("{module}:{line} {reason} `{symbol}`"));
                }
            }
        }
    }
    violations
}

/// Returns the line and name of every publicly declared type, including the
/// entries stamped by the refined-identity macro.
fn declared_type_names(source: &str) -> Vec<(usize, String)> {
    let mut declarations = Vec::new();
    for (line, text) in code_lines(source) {
        let trimmed = text.trim_start();
        let declared = ["pub struct ", "pub enum ", "pub trait ", "pub type "]
            .iter()
            .find_map(|keyword| trimmed.strip_prefix(keyword))
            .or_else(|| macro_stamped_identity(trimmed));

        if let Some(rest) = declared {
            let name = rest
                .trim_start()
                .split(|character: char| !character.is_alphanumeric() && character != '_')
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                declarations.push((line, name.to_owned()));
            }
        }
    }
    declarations
}

/// Recognizes a `Name => Kind, "label";` entry of the refined-identity macro so
/// stamped identities are scanned like ordinary declarations.
fn macro_stamped_identity(trimmed: &str) -> Option<&str> {
    let (name, _) = trimmed.split_once(" => ")?;
    let starts_upper = name.chars().next().is_some_and(char::is_uppercase);
    let is_identifier = name
        .chars()
        .all(|character| character.is_alphanumeric() || character == '_');
    (starts_upper && is_identifier).then_some(trimmed)
}
