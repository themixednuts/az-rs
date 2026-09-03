//! Compose-seam test for `azoth.auth.eos`.
//!
//! A provider gem reaches its host through ADR 0026's `auth_provider` service
//! convention, not through a registry, so the registration surface is empty by
//! design. This pins that: the contribution composes into the `named-service`
//! host with no floor and claims nothing — and the service entry point keeps
//! its own fail-closed behavior beside it.

use az_gem_contract::{Composer, GemTargetRole, ProductActivation};

#[test]
fn the_provider_composes_into_the_named_service_and_claims_nothing() {
    let mut composer = Composer::new(GemTargetRole::NamedService);
    let instance = composer
        .add(
            az_gem_auth_eos::provider_contribution(),
            ProductActivation::default(),
        )
        .expect("the provider declares no capability floor");
    assert_eq!(instance.gem.as_str(), "azoth.auth.eos");
    assert_eq!(instance.contribution.as_str(), "provider");

    let report = composer.finalize().expect("composition is valid");
    assert!(report.refusals.is_empty());
    assert!(
        report.entries.is_empty(),
        "the provider registers nothing; saw {:?}",
        report.entries
    );
}

/// Composition is not wiring: EOS still needs a live verifier from the platform
/// gem, and the entry point still fails closed without one.
#[test]
fn composition_does_not_stand_in_for_the_missing_platform_runtime() {
    let mut composer = Composer::new(GemTargetRole::NamedService);
    composer
        .add(
            az_gem_auth_eos::provider_contribution(),
            ProductActivation::default(),
        )
        .unwrap();
    composer.finalize().expect("composition is valid");

    let context = az_gem_auth::AuthProviderContext::new(
        az_gem_auth::Audience::new("game"),
        az_gem_auth::ProviderId::new(az_gem_auth_eos::PROVIDER_ID),
    );
    assert!(matches!(
        az_gem_auth_eos::auth_provider(&context),
        Err(az_gem_auth::AuthError::HostConfiguration(_))
    ));
}
