//! Compose-seam test for `azoth.legacy-terrain`.
//!
//! Identity comes from `gem.toml` through `#[contribution]`, so what is worth
//! pinning here is that the manifest and the composed descriptor still say the
//! same thing, and that every role the manifest names accepts the contribution
//! rather than refusing it.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_legacy_terrain::package_contribution;
use azoth_legacy_terrain_ids::{GEM, contributions::PACKAGE};

/// The `package` stanza's roles, in manifest order.
const ROLES: [GemTargetRole; 8] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Server,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
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
            .expect("legacy-terrain declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.is_empty(),
            "terrain regions are read from level files, not from a composed registry"
        );
    }
}
