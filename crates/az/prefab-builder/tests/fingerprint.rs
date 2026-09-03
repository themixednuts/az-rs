use az_asset_builder::JobContext;
use az_gem_contract::{
    Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
    GemTargetRole, ProductActivation, Registries, declare_caps,
};
use az_prefab::PrefabType;
use bevy::reflect::Reflect;

#[derive(Reflect)]
struct ContributedPrefabType;

declare_caps!(TestCaps:);

const PREFABS: ContributionDescriptor = ContributionDescriptor {
    gem: GemId::new("azoth.prefab-builder-fingerprint-tests"),
    contribution: ContributionId::new("prefabs"),
    roles: &[GemTargetRole::AssetWorker],
};

struct Prefabs;

impl Contribution for Prefabs {
    type Caps = TestCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        PREFABS
    }

    fn register(&self, context: &mut GemContext<'_, Self::Caps>) {
        context
            .registrar::<PrefabType>()
            .register(PrefabType::of::<ContributedPrefabType>());
    }
}

#[test]
fn public_cook_fingerprint_tracks_composed_prefab_types() {
    let engine_only = Registries::new();
    let mut composed = Composer::new(GemTargetRole::AssetWorker);
    composed
        .add(Prefabs, ProductActivation::default())
        .expect("Prefab contribution composes");

    assert_ne!(
        az_prefab_builder::prefab_cook_analysis_fingerprint(&JobContext::new(&engine_only)),
        az_prefab_builder::prefab_cook_analysis_fingerprint(&JobContext::new(
            composed.registries(),
        )),
        "composed Prefab inputs must change the public cook fingerprint",
    );
}
