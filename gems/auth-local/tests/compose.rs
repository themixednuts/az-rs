//! Compose-seam test for `azoth.auth.local`.
//!
//! A provider gem reaches its host through ADR 0026's `auth_provider` service
//! convention, not through a registry, so the registration surface is empty by
//! design. This pins that: the contribution composes into the `named-service`
//! host with no floor and claims nothing — while the service entry point keeps
//! working beside it.

use az_gem_contract::{Composer, GemTargetRole, ProductActivation};

#[test]
fn the_provider_composes_into_the_named_service_and_claims_nothing() {
    let mut composer = Composer::new(GemTargetRole::NamedService);
    let instance = composer
        .add(
            az_gem_auth_local::provider_contribution(),
            ProductActivation::default(),
        )
        .expect("the provider declares no capability floor");
    assert_eq!(instance.gem.as_str(), "azoth.auth.local");
    assert_eq!(instance.contribution.as_str(), "provider");

    let report = composer.finalize().expect("composition is valid");
    assert!(report.refusals.is_empty());
    assert!(
        report.entries.is_empty(),
        "the provider registers nothing; saw {:?}",
        report.entries
    );
}

/// Composing the gem does not disturb the service convention: the host still
/// builds a provider by calling the entry point, exactly as before.
#[test]
fn composition_leaves_the_service_entry_point_alone() {
    let mut composer = Composer::new(GemTargetRole::NamedService);
    composer
        .add(
            az_gem_auth_local::provider_contribution(),
            ProductActivation::default(),
        )
        .unwrap();
    composer.finalize().expect("composition is valid");

    let context = az_gem_auth::AuthProviderContext::new(
        az_gem_auth::Audience::new("game"),
        az_gem_auth::ProviderId::new(az_gem_auth_local::PROVIDER_ID),
    );
    let provider = az_gem_auth_local::auth_provider(&context).expect("local wires from config");
    assert_eq!(
        provider.capabilities().provider.as_str(),
        az_gem_auth_local::PROVIDER_ID
    );
}
