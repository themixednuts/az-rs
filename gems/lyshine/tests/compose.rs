//! Compose-seam test for `azoth.lyshine`'s `package` contribution.
//!
//! The gem's other contribution, `builders`, lives in the bundle crate beside
//! this one and is tested there — it names `lyshine-canvas`, which depends on
//! this crate, so it could not be composed from here.

use az_core::AzTypeInfo;
use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_lyshine::{UiSpawnerComponent, package_contribution, types};
use azoth_lyshine_ids::{GEM, contributions::PACKAGE};

/// The `package` stanza's roles, in manifest order.
const ROLES: [GemTargetRole; 6] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Unified,
    GemTargetRole::ProjectHost,
    GemTargetRole::RuntimeHost,
];

#[test]
fn the_entry_item_carries_the_manifest_identity() {
    let descriptor = package_contribution().descriptor();

    assert_eq!(descriptor.gem, GEM);
    assert_eq!(descriptor.contribution, PACKAGE);
    assert_eq!(descriptor.roles, ROLES);
}

#[test]
fn every_declared_role_composes_the_spawner_type() {
    for role in ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("lyshine declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(report.entries.len(), types().len());
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );

        assert_eq!(
            az_core::rtti::composed(composer.registries())
                .iter()
                .map(|registration| registration.native_type_id)
                .collect::<Vec<_>>(),
            vec![UiSpawnerComponent::TYPE_ID],
        );
    }
}
