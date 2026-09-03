//! Compose-seam test for `azoth.texture-atlas`'s `builders` contribution.

use az_gem_contract::{Composer, Contribution, GemTargetRole, ProductActivation};
use az_gem_texture_atlas_builders::builders_contribution;
use azoth_texture_atlas_ids::{GEM, contributions::BUILDERS};

/// The `builders` stanza's roles, in manifest order.
const ROLES: [GemTargetRole; 2] = [GemTargetRole::ProjectHost, GemTargetRole::AssetWorker];

#[test]
fn the_entry_item_carries_the_manifest_identity() {
    let descriptor = builders_contribution().descriptor();

    assert_eq!(descriptor.gem, GEM);
    assert_eq!(descriptor.contribution, BUILDERS);
    assert_eq!(descriptor.roles, ROLES);
}

/// The atlas source family, its product format, its asset type and its build
/// rule, reaching a host through the gem that owns the format rather than
/// through link order.
#[test]
fn the_bundle_carries_every_asset_pipeline_registry_the_format_crate_owns() {
    for role in ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(builders_contribution(), ProductActivation::default())
            .expect("texture-atlas declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.iter().all(|entry| entry.instance.gem == GEM),
            "every entry is attributed to this gem"
        );

        for (registry, expected) in [
            ("asset-type", texture_atlas_assets::asset_types().len()),
            (
                "product-format",
                texture_atlas_assets::product_formats().len(),
            ),
            (
                "source-schema",
                texture_atlas_assets::source_schemas().len(),
            ),
            ("build-rule", texture_atlas_assets::build_rules().len()),
        ] {
            let entry_count = report
                .entries
                .iter()
                .filter(|entry| entry.registry == registry)
                .count();
            assert_eq!(entry_count, expected, "`{registry}` under `{role}`");
        }
    }
}
