//! Compose-seam tests for `azoth.lmbr-central`'s `builders` contribution.
//!
//! This is also the only crate that can name all three of the gem's entry
//! items, so the arrangement an asset-worker actually gets — every contribution
//! of one gem in one composition — is asserted here.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_lmbr_central_builders::builders_contribution;
use azoth_lmbr_central_ids::{GEM, contributions::BUILDERS};

/// The `builders` stanza's roles, in manifest order.
const ROLES: [GemTargetRole; 2] = [GemTargetRole::ProjectHost, GemTargetRole::AssetWorker];

#[test]
fn the_entry_item_carries_the_manifest_identity() {
    let descriptor = builders_contribution().descriptor();
    assert_eq!(descriptor.gem, GEM);
    assert_eq!(descriptor.contribution, BUILDERS);
    assert_eq!(descriptor.roles, ROLES);
}

/// The four asset-pipeline registries `lmbr-central-assets` owns, reaching a
/// host through the gem that owns the formats rather than through link order.
#[test]
fn the_bundle_carries_every_asset_pipeline_registry_the_format_crate_owns() {
    for role in ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(builders_contribution(), ProductActivation::default())
            .expect("lmbr-central declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );

        for (registry, expected) in [
            ("asset-type", lmbr_central_assets::asset_types().len()),
            (
                "product-format",
                lmbr_central_assets::product_formats().len(),
            ),
            ("source-schema", lmbr_central_assets::source_schemas().len()),
            ("build-rule", lmbr_central_assets::build_rules().len()),
        ] {
            let actual = report
                .entries
                .iter()
                .filter(|entry| entry.registry == registry)
                .count();
            assert_eq!(actual, expected, "`{registry}` under `{role}`");
        }
    }
}

/// All three of the gem's contributions in one composition: the asset-worker's
/// arrangement, and the proof that three stanzas of one gem claim no key twice.
#[test]
fn the_whole_gem_composes_into_the_asset_worker() {
    let mut composer = Composer::new(GemTargetRole::AssetWorker);
    composer
        .add(
            az_gem_lmbr_central::package_contribution(),
            ProductActivation::default(),
        )
        .expect("lmbr-central declares no capability floor");
    composer
        .add(
            az_gem_lmbr_central::prefab_types_contribution(),
            ProductActivation::default(),
        )
        .expect("lmbr-central declares no capability floor");
    composer
        .add(builders_contribution(), ProductActivation::default())
        .expect("lmbr-central declares no capability floor");

    let report = composer
        .finalize()
        .unwrap_or_else(|error| panic!("the whole gem composes: {error}"));
    assert!(report.refusals.is_empty(), "{report}");
    assert_eq!(report.composed.len(), 3);
    assert!(
        report.entries.iter().all(|entry| entry.instance.gem == GEM),
        "every entry is attributed to this gem"
    );
}
