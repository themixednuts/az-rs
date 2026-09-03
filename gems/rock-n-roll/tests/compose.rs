//! Compose-seam tests for `azoth.rock-n-roll`.
//!
//! The gem declares two contributions out of one package, so beyond the
//! manifest identity there is a second thing worth pinning: the two role sets
//! overlap at `project-host` and `asset-worker`, and a composition that names
//! both there has to stay free of duplicate keys.

use az_core::AzTypeInfo;
use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation, Registries};
use az_gem_rock_n_roll::{
    CharacterControllerComponent, RigidBodyComponent, RockNRollSystemComponent, asset_types,
    package_contribution, prefab_types, prefab_types_contribution, types,
};
use az_prefab::PrefabType;
use azoth_rock_n_roll_ids::{
    GEM,
    contributions::{PACKAGE, PREFAB_TYPES},
};

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
fn the_package_carries_the_component_types_and_the_shape_asset() {
    for role in PACKAGE_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("rock-n-roll declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(report.entries.len(), types().len() + asset_types().len());
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );

        assert_eq!(
            az_core::rtti::composed(composer.registries())
                .iter()
                .map(|registration| registration.native_type_id)
                .collect::<Vec<_>>(),
            vec![
                RigidBodyComponent::TYPE_ID,
                CharacterControllerComponent::TYPE_ID,
                RockNRollSystemComponent::TYPE_ID,
            ],
        );
    }
}

#[test]
fn the_prefab_bundle_carries_every_reflected_type_the_gem_owns() {
    for role in PREFAB_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("rock-n-roll declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(
            paths(composer.registries()),
            prefab_types().map(|entry| entry.path()).to_vec(),
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
            .expect("rock-n-roll declares no capability floor");
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("rock-n-roll declares no capability floor");

        let report = composer
            .finalize()
            .unwrap_or_else(|error| panic!("`{role}` composes both stanzas: {error}"));
        assert_eq!(
            report.entries.len(),
            types().len() + asset_types().len() + prefab_types().len()
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
