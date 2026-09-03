//! Compose-seam tests for `azoth.auth`.
//!
//! The auth family is the pure-capability case: ADR 0026 delivers a provider
//! and a session authority through *service* entry points a generated host
//! calls by name, not through registries. The registration surface is
//! therefore empty on purpose, and these tests are what makes "empty" a
//! checked claim rather than an omission — a future entry appearing here
//! fails, and so does a floor this gem never declared.

use az_gem_contract::{Composer, GemTargetRole, ProductActivation};

/// Every role `gem.toml` gives the `contract` contribution.
const CONTRACT_ROLES: [GemTargetRole; 5] = [
    GemTargetRole::Client,
    GemTargetRole::Server,
    GemTargetRole::Unified,
    GemTargetRole::HeadlessServer,
    GemTargetRole::NamedService,
];

#[test]
fn the_contract_composes_into_every_declared_role_and_claims_nothing() {
    for role in CONTRACT_ROLES {
        let mut composer = Composer::new(role);
        let instance = composer
            .add(
                az_gem_auth::contract_contribution(),
                ProductActivation::default(),
            )
            .expect("the contract declares no capability floor");
        assert_eq!(instance.gem.as_str(), "azoth.auth");
        assert_eq!(instance.contribution.as_str(), "contract");

        let report = composer.finalize().expect("composition is valid");
        assert!(
            report.refusals.is_empty(),
            "the contract is unconditional in {role}"
        );
        assert!(
            report.entries.is_empty(),
            "the contract registers nothing; {role} saw {:?}",
            report.entries
        );
    }
}

/// The host authority is a `named-service` contribution and nothing else: it
/// composes into the generated auth host, and the session signing it owns
/// never becomes registry data.
#[test]
fn the_host_authority_composes_into_the_named_service_and_claims_nothing() {
    let mut composer = Composer::new(GemTargetRole::NamedService);
    let instance = composer
        .add(
            az_gem_auth::host_contribution(),
            ProductActivation::default(),
        )
        .expect("the host authority declares no capability floor");
    assert_eq!(instance.gem.as_str(), "azoth.auth");
    assert_eq!(instance.contribution.as_str(), "host");

    let report = composer.finalize().expect("composition is valid");
    assert!(report.refusals.is_empty());
    assert!(report.entries.is_empty(), "{:?}", report.entries);
}

/// Both contributions of one gem in one host: the generated auth host links
/// the contract and the authority together, and they must not collide.
#[test]
fn the_two_contributions_compose_side_by_side() {
    let mut composer = Composer::new(GemTargetRole::NamedService);
    composer
        .add(
            az_gem_auth::contract_contribution(),
            ProductActivation::default(),
        )
        .unwrap();
    composer
        .add(
            az_gem_auth::host_contribution(),
            ProductActivation::default(),
        )
        .unwrap();

    let report = composer.finalize().expect("composition is valid");
    assert_eq!(report.composed.len(), 2);
    assert!(report.entries.is_empty(), "{:?}", report.entries);
}
