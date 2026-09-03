//! Compose-seam test for `azoth.texture-atlas`'s `package` contribution.
//!
//! The gem's other contribution, `builders`, lives in the bundle crate beside
//! this one and is tested there — it names `texture-atlas-assets`, which
//! depends on this crate, so it could not be composed from here.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_texture_atlas::package_contribution;
use azoth_texture_atlas_ids::{GEM, contributions::PACKAGE};

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
fn every_declared_role_composes_it_without_a_refusal() {
    for role in ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("texture-atlas declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.is_empty(),
            "the atlas registries belong to the `builders` bundle; a new entry \
             here belongs in `register`"
        );
    }
}
