//! Compose-seam test for `azoth.lyshine`'s `builders` contribution.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_lyshine_builders::builders_contribution;
use azoth_lyshine_ids::{GEM, contributions::BUILDERS};

/// The `builders` stanza's roles, in manifest order.
const ROLES: [GemTargetRole; 2] = [GemTargetRole::ProjectHost, GemTargetRole::AssetWorker];

#[test]
fn the_entry_item_carries_the_manifest_identity() {
    let descriptor = builders_contribution().descriptor();

    assert_eq!(descriptor.gem, GEM);
    assert_eq!(descriptor.contribution, BUILDERS);
    assert_eq!(descriptor.roles, ROLES);
}

/// The canvas source family, its product format and its asset type, reaching a
/// host through the gem that owns the format rather than through link order.
///
/// `build-rule` is absent on purpose: `lyshine_canvas::build_rules()` is empty
/// until the `ObjectStream` class capture its job step needs is composable.
#[test]
fn the_bundle_carries_every_asset_pipeline_registry_the_format_crate_owns() {
    for role in ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(builders_contribution(), ProductActivation::default())
            .expect("lyshine declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );

        for (registry, expected) in [
            ("asset-type", lyshine_canvas::asset_types().len()),
            ("product-format", lyshine_canvas::product_formats().len()),
            ("source-schema", lyshine_canvas::source_schemas().len()),
            ("build-rule", lyshine_canvas::build_rules().len()),
        ] {
            let landed = report
                .entries
                .iter()
                .filter(|entry| entry.registry == registry)
                .count();
            assert_eq!(landed, expected, "`{registry}` under `{role}`");
        }
    }
}
