//! Compose-seam tests for `azoth.gradient-signal`.
//!
//! The gem declares two contributions out of one package. Their role sets meet
//! at `project-host`, so beyond the manifest identity what is worth pinning is
//! that the reflected half is claimed once and only by the pipeline bundle.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation, Registries};
use az_gem_gradient_signal::{
    ConstantGradientComponent, GradientTransformComponent, InvertGradientComponent,
    LevelsGradientComponent, PerlinGradientComponent, RandomGradientComponent,
    ThresholdGradientComponent, package_contribution, prefab_types_contribution,
};
use az_prefab::PrefabType;
use azoth_gradient_signal_ids::{
    GEM,
    contributions::{PACKAGE, PREFAB_TYPES},
};
use bevy::reflect::TypePath;

/// The `package` stanza's roles, in manifest order.
const PACKAGE_ROLES: [GemTargetRole; 8] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Server,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
    GemTargetRole::ProjectHost,
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
fn the_package_composes_without_claiming_a_registry() {
    for role in PACKAGE_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(package_contribution(), ProductActivation::default())
            .expect("gradient-signal declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.is_empty(),
            "the reflected half belongs to `prefab-types`, and nothing else here is an entry"
        );
    }
}

#[test]
fn the_prefab_bundle_carries_every_gradient_component() {
    for role in PREFAB_ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(prefab_types_contribution(), ProductActivation::default())
            .expect("gradient-signal declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert_eq!(
            paths(composer.registries()),
            vec![
                ConstantGradientComponent::type_path(),
                ThresholdGradientComponent::type_path(),
                InvertGradientComponent::type_path(),
                LevelsGradientComponent::type_path(),
                RandomGradientComponent::type_path(),
                PerlinGradientComponent::type_path(),
                GradientTransformComponent::type_path(),
            ]
        );
    }
}

/// The two stanzas meet at `project-host`. Composing them together is what
/// proves the split is a split and not two claims on one type.
#[test]
fn the_two_contributions_compose_together_at_project_host() {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(package_contribution(), ProductActivation::default())
        .expect("gradient-signal declares no capability floor");
    composer
        .add(prefab_types_contribution(), ProductActivation::default())
        .expect("gradient-signal declares no capability floor");

    let report = composer
        .finalize()
        .expect("`project-host` composes both stanzas");
    assert_eq!(report.entries.len(), 7, "one entry per gradient component");
    assert!(
        report.entries.iter().all(|entry| entry.instance.gem == GEM),
        "every entry is attributed to this gem"
    );
}

fn paths(registries: &Registries) -> Vec<&'static str> {
    registries
        .get::<PrefabType>()
        .expect("prefab types were registered")
        .entries()
        .map(PrefabType::path)
        .collect()
}
