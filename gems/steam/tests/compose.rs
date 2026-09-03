//! Compose-seam test for `azoth.steam`.
//!
//! Steam's content is settings data, the install locator, and a Steamworks Bevy
//! plugin — none of them a registry entry — so the registration surface is
//! empty. This pins that "empty" is a checked claim: a future entry appearing
//! here fails, and so does a capability floor this gem never declared.

use az_gem_contract::{Composer, GemTargetRole, ProductActivation};

#[test]
fn the_client_runtime_composes_into_every_declared_role() {
    for role in [GemTargetRole::Client, GemTargetRole::Unified] {
        let mut composer = Composer::new(role);
        let instance = composer
            .add(
                az_gem_steam::client_runtime_contribution(),
                ProductActivation::default(),
            )
            .expect("steam declares no capability floor");
        assert_eq!(instance.gem.as_str(), "azoth.steam");
        assert_eq!(instance.contribution.as_str(), "client-runtime");

        let report = composer.finalize().expect("composition is valid");
        assert!(
            report.refusals.is_empty(),
            "steam is unconditional in {role}"
        );
        assert!(
            report.entries.is_empty(),
            "steam registers nothing; {role} saw {:?}",
            report.entries
        );
    }
}
