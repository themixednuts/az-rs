//! Compose-seam tests for `azoth.slayer-script`.
//!
//! The overlap this pins is the one chase-prefab flagged: these ten reflected
//! types were claimed by this gem's `declare_gem!(prefab_types = …)` arm *and*
//! reached a second time through another registry contribution, which link
//! order settled silently. A reflected type gets exactly one owning
//! contribution, the owner is the gem that defines it, and this is the
//! assertion that says so — every composed prefab type carries
//! `azoth.slayer-script` as its attribution.

use az_core::rtti::AzTypeRegistration;
use az_gem_contract::{
    ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
    GemId, GemTargetRole, ProductActivation, declare_caps,
};
use az_gem_slayer_script::{
    SLAYER_SCRIPT_SYSTEM_COMPONENT_TYPE_UUID, package_contribution, prefab_types,
    prefab_types_contribution,
};
use az_prefab::PrefabType;

const GEM: &str = "azoth.slayer-script";

/// Every role `gem.toml` gives the `package` contribution.
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

#[test]
fn the_package_composes_into_every_declared_role() {
    for role in PACKAGE_ROLES {
        let mut composer = Composer::new(role);
        let instance = composer
            .add(package_contribution(), ProductActivation::default())
            .expect("slayer script declares no capability floor");
        assert_eq!(instance.gem.as_str(), GEM);
        assert_eq!(instance.contribution.as_str(), "package");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "unconditional in {role}");
        assert_eq!(
            report.entries.len(),
            1,
            "the system component's AZ type, and nothing else, in {role}"
        );
        assert_eq!(report.entries[0].registry, "az-type");
    }
}

#[test]
fn the_system_component_composes_under_its_native_type_id() {
    let mut composer = Composer::new(GemTargetRole::RuntimeHost);
    composer
        .add(package_contribution(), ProductActivation::default())
        .unwrap();

    let types = composer
        .registries()
        .get::<AzTypeRegistration>()
        .expect("the AZ type was registered");
    assert_eq!(types.len(), 1);
    assert_eq!(
        types.entries().next().unwrap().native_type_id,
        SLAYER_SCRIPT_SYSTEM_COMPONENT_TYPE_UUID
    );
}

fn worker() -> Composer {
    let mut composer = Composer::new(GemTargetRole::AssetWorker);
    composer
        .add(prefab_types_contribution(), ProductActivation::default())
        .expect("the prefab types declare no capability floor");
    composer
}

/// The attribution proof. Ten entries, one per type this gem defines, every one
/// of them owned by `azoth.slayer-script`.
#[test]
fn every_composed_prefab_type_is_attributed_to_this_gem() {
    let composer = worker();
    let registry = composer
        .registries()
        .get::<PrefabType>()
        .expect("the prefab types were registered");

    assert_eq!(registry.len(), prefab_types().len());
    for attributed in registry {
        assert_eq!(
            attributed.instance.gem.as_str(),
            GEM,
            "`{}` is claimed by the gem that defines it",
            attributed.entry.path()
        );
        assert_eq!(attributed.instance.contribution.as_str(), "prefab-types");
    }

    let composed_paths = registry.entries().map(PrefabType::path).collect::<Vec<_>>();
    let declared_paths = prefab_types()
        .iter()
        .map(az_prefab::PrefabType::path)
        .collect::<Vec<_>>();
    assert_eq!(
        composed_paths, declared_paths,
        "the whole declared set composed"
    );
}

/// Applying the composed entries is what an asset worker does with them, and
/// the prefab metadata has to survive that round trip — this is the assertion
/// the deleted `register_prefab_types` unit test carried, now made against a
/// composed host instead of a hand-built registry.
#[test]
fn applying_the_composed_types_carries_prefab_type_data() {
    let composer = worker();
    let mut registry = bevy::reflect::TypeRegistry::default();
    for entry in composer
        .registries()
        .get::<PrefabType>()
        .expect("the prefab types were registered")
        .entries()
    {
        entry.apply(&mut registry);
    }

    assert!(
        registry
            .get(std::any::TypeId::of::<
                az_gem_slayer_script::SlayerScriptSystemComponent,
            >())
            .and_then(|registration| registration.data::<az_prefab::PrefabTypeData>())
            .is_some(),
        "the composed system component still carries its prefab type data"
    );
}

declare_caps!(RivalCaps:);

/// A second gem claiming this gem's reflected types — the shape of the overlap
/// the sweep resolved.
struct Rival;

impl Contribution for Rival {
    type Caps = RivalCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.test-rival"),
            contribution: ContributionId::new("prefab-types"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, RivalCaps>) {
        ctx.registrar::<PrefabType>().register_many(prefab_types());
    }
}

/// Two contributions claiming one reflected type is now a composition error
/// naming both, where link order used to pick one and say nothing.
#[test]
fn a_second_gem_claiming_these_types_fails_composition() {
    let mut composer = worker();
    composer.add(Rival, ProductActivation::default()).unwrap();

    let ComposeError::Duplicate {
        registry,
        key,
        first,
        second,
    } = composer.finalize().unwrap_err()
    else {
        panic!("expected a duplicate registry key");
    };
    assert_eq!(registry, "prefab-type");
    assert_eq!(key, prefab_types()[0].path());
    assert_eq!(first.gem.as_str(), GEM);
    assert_eq!(second.gem.as_str(), "azoth.test-rival");
}

#[test]
fn withdrawing_the_contribution_takes_all_ten_types_with_it() {
    let mut composer = worker();
    let removed = composer.remove(GemId::new(GEM), ContributionId::new("prefab-types"));
    assert_eq!(removed, prefab_types().len());

    let report = composer.finalize().expect("composition is valid");
    assert!(report.entries.is_empty());
}
