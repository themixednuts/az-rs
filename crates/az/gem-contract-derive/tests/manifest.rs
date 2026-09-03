//! The manifest forms of `#[contribution]` against real manifests.
//!
//! `tests/mount` is a gem the way a gem is actually laid out — a `gem.toml` at
//! the root, one contribution crate beside it implementing two of the gem's
//! contributions, others below it implementing one each — so these are
//! assertions about a real compilation, not about a token stream. A
//! manifest-declared contribution and a hand-declared one are the same
//! composition, and the manifest is a tracked input of the crate that reads it.
//!
//! `tests/engine` is the same layout with `engine.toml` at its root. The
//! engine is a manifested composition root rather than a gem, and everything
//! below asserts the consequence: one reader, one grammar, one set of
//! diagnostics, and identity that an engine author never spells.

use std::path::{Path, PathBuf};

use az_gem_contract::prelude::*;
use az_gem_contract::{CapSet, CapsDecl, ComposeReport, Composer, ProductActivation};

const ROLES: [GemTargetRole; 5] = [
    GemTargetRole::Server,
    GemTargetRole::Client,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
    GemTargetRole::AssetWorker,
];

fn compose<C: Contribution>(role: GemTargetRole, contribution: C) -> ComposeReport {
    let mut composer = Composer::new(role);
    let _ = composer.add(contribution, ProductActivation::default());
    composer.finalize().expect("composition is valid")
}

/// The entry item is the only way in: `Runtime` is private to its gem.
fn runtime() -> impl Contribution {
    az_gem_mount::runtime_contribution()
}

#[test]
fn the_manifest_resolves_the_same_descriptor_the_keys_spell() {
    assert_eq!(runtime().descriptor(), az_gem_mount::Keyed.descriptor());
    assert_eq!(runtime().descriptor().gem, az_gem_mount::ids::GEM);
    assert_eq!(
        runtime().descriptor().contribution,
        az_gem_mount::ids::RUNTIME
    );
    assert!(
        runtime()
            .descriptor()
            .applies_to_role(GemTargetRole::Unified)
    );
    assert!(
        !runtime()
            .descriptor()
            .applies_to_role(GemTargetRole::Client)
    );
}

#[test]
fn the_manifests_caps_are_the_same_floor_the_keys_declare() {
    let floor = CapSet::EMPTY
        .with(HostCapability::HasAuthority)
        .with(HostCapability::HostsWorld);
    assert_eq!(az_gem_mount::RuntimeCaps::REQUIRED, floor);
    assert_eq!(
        az_gem_mount::RuntimeCaps::REQUIRED,
        az_gem_mount::KeyedCaps::REQUIRED
    );
}

#[test]
fn both_forms_compose_identically_in_every_role() {
    for role in ROLES {
        let manifest = compose(role, runtime());
        let keyed = compose(role, az_gem_mount::Keyed);
        assert_eq!(manifest, keyed, "`{role}` composes the same either way");
        assert_eq!(manifest.to_string(), keyed.to_string());
    }
}

#[test]
fn the_declared_floor_is_enforced_by_the_witness() {
    let server = compose(GemTargetRole::Server, runtime());
    assert!(server.refusals.is_empty());
    assert_eq!(server.declined_narrowings.len(), 1);

    let client = compose(GemTargetRole::Client, runtime());
    assert_eq!(client.refusals.len(), 1);
    assert!(client.entries.is_empty());
    let refusal = client.refusals[0].to_string();
    assert!(refusal.contains("azoth.mount"), "{refusal}");
    assert!(refusal.contains("HasAuthority"), "{refusal}");
}

/// Two contributions, one package: each block names itself, and the two entry
/// items are two identities with two floors and two role sets — everything the
/// package alone could not have told them apart by.
#[test]
fn a_bare_token_selects_among_one_packages_contributions() {
    let host = az_gem_mount::runtime_host_contribution();
    assert_eq!(host.descriptor().gem, az_gem_mount::ids::GEM);
    assert_eq!(host.descriptor().contribution.as_str(), "runtime-host");
    assert_eq!(host.descriptor().roles, &[GemTargetRole::RuntimeHost]);
    assert_eq!(az_gem_mount::HostCaps::REQUIRED, CapSet::EMPTY);

    assert_ne!(
        runtime().descriptor().contribution,
        host.descriptor().contribution
    );

    let report = compose(GemTargetRole::RuntimeHost, host);
    assert!(report.refusals.is_empty());
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].key, "MountHostPanel");
}

/// A crate one level below the manifest, a hyphenated id, and a stanza with no
/// `caps` at all: the fold to `prefab_types_contribution` and the empty floor
/// are both the manifest's doing.
#[test]
fn a_nested_crate_resolves_the_gem_above_it() {
    let types = az_gem_mount_prefab::prefab_types_contribution();
    let descriptor = types.descriptor();
    assert_eq!(descriptor.gem, az_gem_mount::ids::GEM);
    assert_eq!(descriptor.contribution.as_str(), "prefab-types");
    assert_eq!(descriptor.roles, &[GemTargetRole::AssetWorker]);
    assert_eq!(az_gem_mount_prefab::TypesCaps::REQUIRED, CapSet::EMPTY);

    let report = compose(GemTargetRole::AssetWorker, types);
    assert!(report.refusals.is_empty());
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].key, "MountComponent");
}

/// The engine takes its identity from `engine.toml` through the same reader:
/// `[engine] id` answers where `[gem] id` would, and the entry item, the role
/// set, and the floor all arrive exactly as a gem's do.
#[test]
fn an_engine_crate_resolves_the_engine_manifest_above_it() {
    let builders = az_engine_mount::builders_contribution();
    let descriptor = builders.descriptor();
    assert_eq!(descriptor.gem.as_str(), "azoth");
    assert_eq!(descriptor.contribution.as_str(), "builders");
    assert_eq!(
        descriptor.roles,
        &[
            GemTargetRole::AssetProcessor,
            GemTargetRole::AssetWorker,
            GemTargetRole::ProjectHost,
        ]
    );
    assert_eq!(
        az_engine_mount::BuildersCaps::REQUIRED,
        CapSet::EMPTY,
        "an omitted floor is the empty floor, in either manifest"
    );

    let report = compose(GemTargetRole::AssetProcessor, builders);
    assert!(report.refusals.is_empty());
    assert_eq!(report.entries.len(), 1);
    assert_eq!(report.entries[0].key, "mount-source");
    assert_eq!(report.composed[0].gem.as_str(), "azoth");
}

/// A crate one level below `engine.toml`, a hyphenated id, and a declared
/// floor the witness enforces — the same facts the nested *gem* crate proves,
/// from the other manifest.
#[test]
fn a_nested_engine_crate_resolves_the_engine_above_it() {
    let descriptor = az_engine_mount_types::prefab_types_contribution().descriptor();
    assert_eq!(descriptor.gem.as_str(), "azoth");
    assert_eq!(descriptor.contribution.as_str(), "prefab-types");
    assert_eq!(
        az_engine_mount_types::TypesCaps::REQUIRED,
        CapSet::EMPTY.with(HostCapability::HostsWorld)
    );

    let client = compose(
        GemTargetRole::Client,
        az_engine_mount_types::prefab_types_contribution(),
    );
    assert!(client.refusals.is_empty());
    assert_eq!(client.entries.len(), 1);
    assert_eq!(client.entries[0].key, "MountTransform");

    // The floor is enforced the same way whichever manifest declared it, and
    // the refusal names the engine as it would name a gem.
    let host = compose(
        GemTargetRole::ProjectHost,
        az_engine_mount_types::prefab_types_contribution(),
    );
    assert_eq!(host.refusals.len(), 1);
    assert!(host.entries.is_empty());
    let refusal = host.refusals[0].to_string();
    assert!(refusal.contains("azoth"), "{refusal}");
    assert!(refusal.contains("HostsWorld"), "{refusal}");
}

/// A declared generated product is registered without an author writing the
/// call.
///
/// `gem.toml` says `products = ["graphs"]`, the fixture's build script writes
/// the projection into `OUT_DIR`, and the crate's `register` body mentions
/// neither. The AOT graph below is in the composition anyway, ahead of the
/// body's own entry — which is the ordering the expansion guarantees.
#[test]
fn a_declared_product_composes_without_a_registration_line() {
    let graphs = az_gem_mount_graphs::graphs_contribution();
    assert_eq!(graphs.descriptor().contribution.as_str(), "graphs");

    let mut composer = Composer::new(GemTargetRole::Server);
    let _ = composer.add(graphs, ProductActivation::default());
    let report = composer.finalize().expect("composition is valid");
    assert!(report.refusals.is_empty(), "{report}");

    let keys: Vec<(&str, &str)> = report
        .entries
        .iter()
        .map(|entry| (entry.registry, entry.key.as_str()))
        .collect();
    assert_eq!(
        keys,
        [
            ("aot-graph", "az-gem-mount-graphs::mount_graph"),
            ("marker", "MountGraphsRan"),
        ],
        "the projection registers first, ahead of the author's own body"
    );

    // The registry it landed in is the graph runtime's own, which is how a
    // host resolves an AOT graph — not a look-alike the fixture invented.
    let registered_graphs = az_graph_runtime::aot_graphs(composer.registries());
    assert_eq!(registered_graphs.len(), 1);
    assert!(registered_graphs[0].matches("az-gem-mount-graphs", "mount_graph"));
}

/// Editing `gem.toml` rebuilds the crate that reads it.
///
/// The expansion `include_bytes!`es the manifest, so rustc lists it as an input
/// of the contribution crate and Cargo folds it into that crate's fingerprint.
/// This asserts the mechanism at its source: the dep-info rustc emitted for the
/// fixture names the manifest.
#[test]
fn the_gem_manifest_is_a_tracked_input_of_the_contribution_crate() {
    let deps = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .expect("the test binary lives in the deps directory");

    let tracking: Vec<PathBuf> = std::fs::read_dir(&deps)
        .expect("the deps directory is readable")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|extension| extension == "d")
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("az_gem_mount-"))
        })
        .collect();
    assert!(
        !tracking.is_empty(),
        "no dep-info for the fixture gem in {}",
        deps.display()
    );

    for path in tracking {
        let text = std::fs::read_to_string(&path)
            .expect("dep-info is readable")
            .replace('\\', "/");
        assert!(
            text.contains("tests/mount/gem.toml"),
            "`{}` does not track the gem manifest:\n{text}",
            path.display()
        );
    }
}
