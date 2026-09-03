//! Compose-seam test for `azoth.animation`.
//!
//! Identity comes from `gem.toml` through `#[contribution]`, so what is worth
//! pinning here is that the manifest and the composed descriptor still say the
//! same thing, and that every role the manifest names accepts the contribution
//! rather than refusing it.

use az_gem_animation::package_contribution;
use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use azoth_animation_ids::{GEM, contributions::PACKAGE};

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
            .expect("animation declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.is_empty(),
            "the animation pipeline entries belong to `az-animation`, composed by the engine"
        );
    }
}

/// `project-host` is where this gem's roles meet the engine's `builders`
/// bundle, and the reason `az_animation::register` is called from exactly one
/// of them: composing both is a valid composition only while this one stays
/// out of that crate's registries.
#[test]
fn the_engine_builders_bundle_composes_beside_it_at_project_host() {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(
            az_engine_builders::builders_contribution(),
            ProductActivation::default(),
        )
        .expect("the engine bundle declares no capability floor");
    composer
        .add(package_contribution(), ProductActivation::default())
        .expect("animation declares no capability floor");

    let report = composer
        .finalize()
        .expect("the gem claims nothing the engine bundle already claimed");
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.instance.gem == az_engine_ids::ENGINE),
        "every entry composed here is the engine's"
    );
}
