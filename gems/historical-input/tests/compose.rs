//! Compose-seam tests for `azoth.historical-input`.
//!
//! The three entries this gem owns — the input-map asset type, its AZ type, and
//! the system component's lowering — used to reach a host through a linked
//! `inventory` submission and two derive-emitted submissions, which no host
//! could enumerate or attribute. They are now read back off a composed host's
//! own registries, attributed to the contributing gem.

use az_core::{AssetTypeRegistration, ComponentLoweringRegistration, rtti::AzTypeRegistration};
use az_gem_contract::{
    ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
    GemId, GemTargetRole, ProductActivation, declare_caps,
};
use az_gem_historical_input::{
    HISTORICAL_INPUT_SYSTEM_COMPONENT_TYPE_ID, INPUT_MAP_ASSET_STABLE_NAME,
    INPUT_MAP_ASSET_TYPE_ID, package_contribution,
};

const GEM: &str = "azoth.historical-input";

/// Every role `gem.toml` gives the `package` contribution.
const ROLES: [GemTargetRole; 6] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Unified,
    GemTargetRole::ProjectHost,
    GemTargetRole::RuntimeHost,
];

fn compose(role: GemTargetRole) -> Composer {
    let mut composer = Composer::new(role);
    composer
        .add(package_contribution(), ProductActivation::default())
        .expect("historical input declares no capability floor");
    composer
}

#[test]
fn the_package_composes_into_every_declared_role() {
    for role in ROLES {
        let report = compose(role).finalize().expect("composition is valid");
        assert!(
            report.refusals.is_empty(),
            "historical input is unconditional in {role}"
        );
        assert_eq!(
            report.entries.len(),
            3,
            "one asset type, one AZ type, one lowering in {role}; saw {:?}",
            report.entries
        );
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.instance.gem.as_str() == GEM),
            "every entry is attributed to the contributing gem in {role}"
        );
    }
}

#[test]
fn the_input_map_asset_type_composes_under_its_stable_name() {
    let composer = compose(GemTargetRole::RuntimeHost);
    let asset_types = composer
        .registries()
        .get::<AssetTypeRegistration>()
        .expect("the asset type was registered");

    assert_eq!(asset_types.len(), 1);
    let registration = asset_types.entries().next().unwrap();
    assert_eq!(registration.stable_name(), INPUT_MAP_ASSET_STABLE_NAME);
    assert_eq!(registration.asset_type().0, INPUT_MAP_ASSET_TYPE_ID);
    assert_eq!(
        registration.owner(),
        "az-gem-historical-input",
        "the owner string the deleted submission carried survives as data"
    );
}

/// The two derive-emitted registrations: the derive now names a const, and this
/// is where the crate's enumeration of those consts is proved to reach a host.
#[test]
fn the_derived_type_and_lowering_compose_under_their_native_ids() {
    let composer = compose(GemTargetRole::RuntimeHost);
    let registries = composer.registries();

    let types = registries
        .get::<AzTypeRegistration>()
        .expect("the AZ type was registered");
    assert_eq!(types.len(), 1);
    assert_eq!(
        types.entries().next().unwrap().native_type_id,
        INPUT_MAP_ASSET_TYPE_ID
    );

    let lowerings = registries
        .get::<ComponentLoweringRegistration>()
        .expect("the lowering was registered");
    assert_eq!(lowerings.len(), 1);
    assert_eq!(
        lowerings
            .entries()
            .next()
            .unwrap()
            .type_registration
            .native_type_id,
        HISTORICAL_INPUT_SYSTEM_COMPONENT_TYPE_ID
    );
}

declare_caps!(RivalCaps:);

/// A second gem claiming the same asset type id.
struct Rival;

impl Contribution for Rival {
    type Caps = RivalCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("azoth.rival"),
            contribution: ContributionId::new("package"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, RivalCaps>) {
        ctx.registrar::<AssetTypeRegistration>()
            .register_many(az_gem_historical_input::asset_types());
    }
}

/// The asset type id is what a product carries: two gems claiming it disagree
/// about what those bytes are, which link order used to settle silently.
#[test]
fn two_gems_claiming_the_input_map_asset_type_fail_composition() {
    let mut composer = compose(GemTargetRole::RuntimeHost);
    composer.add(Rival, ProductActivation::default()).unwrap();

    let ComposeError::Duplicate {
        registry,
        first,
        second,
        ..
    } = composer.finalize().unwrap_err()
    else {
        panic!("expected a duplicate registry key");
    };
    assert_eq!(registry, "asset-type");
    assert_eq!(first.gem.as_str(), GEM);
    assert_eq!(second.gem.as_str(), "azoth.rival");
}

#[test]
fn withdrawing_the_gem_takes_all_three_registries_with_it() {
    let mut composer = compose(GemTargetRole::RuntimeHost);
    let removed = composer.remove(GemId::new(GEM), ContributionId::new("package"));
    assert_eq!(removed, 3, "the asset type, the AZ type, and the lowering");

    let report = composer.finalize().expect("composition is valid");
    assert!(report.entries.is_empty());
}
