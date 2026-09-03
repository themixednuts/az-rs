//! Compose-seam test for `azoth.footsteps`.
//!
//! Identity comes from `gem.toml` through `#[contribution]`, so what is worth
//! pinning here is that the manifest and the composed descriptor still say the
//! same thing, and that every role the manifest names accepts the contribution
//! rather than refusing it.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_footsteps::package_contribution;
use azoth_footsteps_ids::{GEM, contributions::PACKAGE};

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
            .expect("footsteps declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.is_empty(),
            "footsteps owns no registry entries; a new one belongs in `register`"
        );
    }
}
