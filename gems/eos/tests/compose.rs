//! Compose-seam tests for `azoth.eos`.
//!
//! The platform gem's content is an SDK-backed Bevy plugin, and a plugin is not
//! a registry entry, so the registration surface is empty. What is worth pinning
//! is the shape the manifest declares: three separately selectable
//! contributions, each with its own role set, that compose together into a
//! `unified` host without colliding.

use az_gem_contract::{Composer, GemTargetRole, ProductActivation};

#[test]
fn the_client_runtime_composes_into_its_declared_roles() {
    for role in [GemTargetRole::Client, GemTargetRole::Unified] {
        let mut composer = Composer::new(role);
        let instance = composer
            .add(
                az_gem_eos::client_runtime_contribution(),
                ProductActivation::default(),
            )
            .expect("the client runtime declares no capability floor");
        assert_eq!(instance.gem.as_str(), "azoth.eos");
        assert_eq!(instance.contribution.as_str(), "client-runtime");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "unconditional in {role}");
        assert!(report.entries.is_empty(), "{:?}", report.entries);
    }
}

#[test]
fn the_server_runtime_composes_into_its_declared_roles() {
    for role in [
        GemTargetRole::Server,
        GemTargetRole::Unified,
        GemTargetRole::HeadlessServer,
    ] {
        let mut composer = Composer::new(role);
        let instance = composer
            .add(
                az_gem_eos::server_runtime_contribution(),
                ProductActivation::default(),
            )
            .expect("the server runtime declares no capability floor");
        assert_eq!(instance.contribution.as_str(), "server-runtime");

        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "unconditional in {role}");
        assert!(report.entries.is_empty(), "{:?}", report.entries);
    }
}

/// The three contributions are separate instances of one gem, so a `unified`
/// host that selected all of them composes three and collides on nothing.
#[test]
fn the_three_contributions_compose_side_by_side() {
    let mut composer = Composer::new(GemTargetRole::Unified);
    composer
        .add(
            az_gem_eos::client_runtime_contribution(),
            ProductActivation::default(),
        )
        .unwrap();
    composer
        .add(
            az_gem_eos::anti_cheat_client_contribution(),
            ProductActivation::default(),
        )
        .unwrap();
    composer
        .add(
            az_gem_eos::server_runtime_contribution(),
            ProductActivation::default(),
        )
        .unwrap();

    let report = composer.finalize().expect("composition is valid");
    assert_eq!(report.composed.len(), 3);
    assert!(
        report
            .composed
            .iter()
            .all(|instance| instance.gem.as_str() == "azoth.eos"),
        "every instance is this gem's"
    );
    assert!(report.entries.is_empty(), "{:?}", report.entries);
}
