//! Compose-seam tests for the engine's `runtime` bundle.
//!
//! The bundle's reason to exist is attribution: the engine's adapters and
//! reflected types used to arrive outside composition, so nothing could say who
//! owned them and nothing could catch a second claim. These tests hold the
//! middleware's seven reflected types to that — composed, keyed on type path,
//! and attributed to `azoth`/`runtime` at every role the manifest names.

use az_core::{AzTypeInfo, rtti::AzTypeRegistration};
use az_engine_ids::{ENGINE, contributions::RUNTIME};
use az_engine_runtime::runtime_contribution;
use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation, RegistryEntry};
use az_prefab::PrefabType;

/// The `runtime` stanza's roles, in `engine.toml` order.
const ROLES: [GemTargetRole; 9] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Server,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
    GemTargetRole::AssetProcessor,
    GemTargetRole::AssetWorker,
    GemTargetRole::RuntimeHost,
];

/// The three roles the stanza leaves out: two that name no world, and the
/// service adapter that may not link gridmate.
const OUTSIDERS: [GemTargetRole; 3] = [
    GemTargetRole::ProjectHost,
    GemTargetRole::Tool,
    GemTargetRole::NamedService,
];

/// The type paths az-facet owns, in the order it lists them.
fn paths() -> Vec<&'static str> {
    az_facet::prefab_types()
        .iter()
        .map(PrefabType::path)
        .collect()
}

fn compose(role: GemTargetRole) -> Composer {
    let mut composer = Composer::new(role);
    composer
        .floor(runtime_contribution())
        .expect("the engine floor declares no host-capability floor");
    composer
        .finalize()
        .expect("the engine composition is valid");
    composer
}

#[test]
fn the_entry_item_carries_the_manifest_identity() {
    let descriptor = runtime_contribution().descriptor();
    assert_eq!(descriptor.gem, ENGINE);
    assert_eq!(descriptor.contribution, RUNTIME);
    assert_eq!(descriptor.roles, ROLES);
}

#[test]
fn every_role_the_stanza_names_gets_the_middlewares_reflected_types() {
    let expected = paths();
    assert_eq!(expected.len(), 9, "az-facet owns nine reflected types");
    assert!(expected.contains(&PrefabType::of::<az_facet::LocalComponentRefBase>().path()));
    assert!(expected.contains(&PrefabType::of::<az_facet::RemoteTypelessServerFacetRef>().path()));

    for role in ROLES {
        let composer = compose(role);
        let registry = composer
            .registries()
            .get::<PrefabType>()
            .expect("the runtime bundle registers reflected types");
        let az_types = composer
            .registries()
            .get::<AzTypeRegistration>()
            .expect("the runtime bundle registers native AZ types");

        assert_eq!(
            registry.entries().map(PrefabType::path).collect::<Vec<_>>(),
            expected,
            "reflected types under `{role}`"
        );
        assert!(
            registry.iter().all(|attributed| {
                attributed.instance.gem == ENGINE && attributed.instance.contribution == RUNTIME
            }),
            "every reflected type is attributed to the engine under `{role}`"
        );
        assert!(
            az_types.entries().any(|registration| {
                registration.native_type_id == az_facet::LocalComponentRefBase::TYPE_ID
                    && (registration.rust_type_id)()
                        == std::any::TypeId::of::<az_facet::LocalComponentRefBase>()
            }),
            "the native local-component reference is composed under `{role}`"
        );
    }
}

#[test]
fn the_report_names_the_engine_for_each_reflected_type() {
    let mut composer = Composer::new(GemTargetRole::AssetWorker);
    composer
        .add(runtime_contribution(), ProductActivation::default())
        .expect("the engine floor declares no host-capability floor");
    let report = composer
        .finalize()
        .expect("the engine composition is valid");
    assert!(report.refusals.is_empty(), "the bundle is unconditional");

    let reflected = report
        .entries
        .iter()
        .filter(|entry| entry.registry == PrefabType::registry_name())
        .collect::<Vec<_>>();

    assert_eq!(
        reflected
            .iter()
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>(),
        paths(),
    );
    assert!(
        reflected
            .iter()
            .all(|entry| entry.instance.gem == ENGINE && entry.instance.contribution == RUNTIME),
        "the report attributes every reflected type to the engine"
    );
}

#[test]
fn the_roles_the_stanza_skips_take_none_of_it() {
    for role in OUTSIDERS {
        let composer = compose(role);
        assert!(
            composer.registries().get::<PrefabType>().is_none(),
            "`{role}` composes no engine reflected types"
        );
    }
}
