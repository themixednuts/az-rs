//! Compose-seam tests for `azoth.lmbr-central`'s two in-package contributions.
//!
//! The gem's third contribution, `builders`, lives in the bundle crate beside
//! this one and is tested there — it names `lmbr-central-assets`, which depends
//! on this crate, so it could not be composed from here.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation, Registries};
use az_gem_lmbr_central::{
    lowerings, package_contribution, prefab_types, prefab_types_contribution, types,
};
use az_prefab::PrefabType;
use azoth_lmbr_central_ids::{
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
fn the_package_carries_every_az_type_and_every_lowering() {
    for role in PACKAGE_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("lmbr-central declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(report.entries.len(), types().len() + lowerings().len());
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );

        let registries = composer.registries();
        assert_eq!(
            az_core::component::lowering::composed(registries).len(),
            lowerings().len()
        );
        assert_eq!(
            az_core::rtti::composed(registries).len(),
            types().len() + lowerings().len(),
            "the direct registrations plus the type half of each lowering"
        );
    }
}

#[test]
fn the_prefab_bundle_carries_every_reflected_type_the_gem_owns() {
    for role in PREFAB_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("lmbr-central declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(
            paths(composer.registries()),
            prefab_types().map(|entry| entry.path()).to_vec(),
        );
    }
}

/// Every reflected type the gem hands a host resolves as a Prefab type — the
/// assertion the deleted `prefab_registration_is_pure_and_complete` made
/// against a `TypeRegistry`, now made against a composition.
#[test]
fn every_composed_prefab_type_carries_prefab_type_data() {
    let mut composer = Composer::new(GemTargetRole::AssetWorker);
    composer
        .add(prefab_types_contribution(), ProductActivation::default())
        .expect("lmbr-central declares no capability floor");

    let mut registry = bevy::reflect::TypeRegistry::default();
    for entry in composer
        .registries()
        .get::<PrefabType>()
        .expect("prefab types were registered")
        .entries()
    {
        entry.apply(&mut registry);
    }

    for entry in prefab_types() {
        assert!(
            registry
                .get_with_type_path(entry.path())
                .and_then(|registration| registration.data::<az_prefab::PrefabTypeData>())
                .is_some(),
            "`{}` must carry PrefabTypeData",
            entry.path()
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
            .expect("lmbr-central declares no capability floor");
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("lmbr-central declares no capability floor");

        let report = composer
            .finalize()
            .unwrap_or_else(|error| panic!("`{role}` composes both stanzas: {error}"));
        assert_eq!(
            report.entries.len(),
            types().len() + lowerings().len() + prefab_types().len()
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
