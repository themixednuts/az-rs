//! Compose-seam tests for `azoth.fast-noise`.
//!
//! The gem declares two contributions out of one package, so beyond the
//! manifest identity there is a second thing worth pinning: the two role sets
//! overlap at `project-host` and `asset-worker`, and a composition that names
//! both there has to stay free of duplicate keys.

use az_core::AzTypeInfo;
use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation, Registries};
use az_gem_fast_noise::{
    FastNoiseGradientComponent, FastNoiseGradientConfig, package_contribution,
    prefab_types_contribution,
};
use az_prefab::PrefabType;
use azoth_fast_noise_ids::{
    GEM,
    contributions::{PACKAGE, PREFAB_TYPES},
};
use bevy::reflect::TypePath;

/// The `package` stanza's roles, in manifest order.
const PACKAGE_ROLES: [GemTargetRole; 9] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Server,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
    GemTargetRole::ProjectHost,
    GemTargetRole::AssetWorker,
    GemTargetRole::RuntimeHost,
];

/// The `prefab-types` stanza's roles, in manifest order.
const PREFAB_ROLES: [GemTargetRole; 2] = [GemTargetRole::ProjectHost, GemTargetRole::AssetWorker];

#[test]
fn the_entry_items_carry_the_manifest_identities() {
    let package = package_contribution().descriptor();
    assert_eq!(package.gem, GEM);
    assert_eq!(package.contribution, PACKAGE);
    assert_eq!(package.roles, PACKAGE_ROLES);

    let prefabs = prefab_types_contribution().descriptor();
    assert_eq!(prefabs.gem, GEM);
    assert_eq!(prefabs.contribution, PREFAB_TYPES);
    assert_eq!(prefabs.roles, PREFAB_ROLES);
}

#[test]
fn the_package_carries_the_gradient_type_and_its_lowering() {
    for role in PACKAGE_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("fast-noise declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");

        let registries = composer.registries();
        assert_eq!(
            az_core::rtti::composed(registries)
                .iter()
                .map(|registration| registration.native_type_id)
                .collect::<Vec<_>>(),
            vec![
                FastNoiseGradientConfig::TYPE_ID,
                FastNoiseGradientComponent::TYPE_ID,
            ],
            "the config registers directly, the component through its lowering"
        );
        assert_eq!(az_core::component::lowering::composed(registries).len(), 1);
    }
}

#[test]
fn the_prefab_bundle_carries_the_gradient_component() {
    for role in PREFAB_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("fast-noise declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(
            paths(composer.registries()),
            vec![FastNoiseGradientComponent::type_path()]
        );
    }
}

/// The two stanzas meet at `project-host` and `asset-worker`. Composing them
/// together is what proves the split is a split and not two claims on one type.
#[test]
fn the_two_contributions_compose_together_where_their_roles_overlap() {
    for role in PREFAB_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("fast-noise declares no capability floor");
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("fast-noise declares no capability floor");

        let report = composer
            .finalize()
            .unwrap_or_else(|error| panic!("`{role}` composes both stanzas: {error}"));
        assert_eq!(
            report.entries.len(),
            3,
            "one AZ type, one lowering, one prefab type"
        );
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );
    }
}

fn paths(registries: &Registries) -> Vec<&'static str> {
    registries
        .get::<PrefabType>()
        .expect("prefab types were registered")
        .entries()
        .map(PrefabType::path)
        .collect()
}
