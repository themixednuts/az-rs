//! Compose-seam test for `azoth.camera`.
//!
//! The gem's content is a Bevy plugin, which is not a registry entry, so what
//! this pins is that the contribution composes cleanly into every role its
//! manifest names and claims nothing — the deletion of `declare_gem!` took a
//! zero-registration declaration with it, and this is the assertion that says
//! zero was the honest number.

use az_gem_contract::{Composer, GemTargetRole, ProductActivation};

const ROLES: [GemTargetRole; 6] = [
    GemTargetRole::Game,
    GemTargetRole::P2p,
    GemTargetRole::Client,
    GemTargetRole::Unified,
    GemTargetRole::ProjectHost,
    GemTargetRole::RuntimeHost,
];

#[test]
fn the_package_contribution_composes_into_every_declared_role() {
    for role in ROLES {
        let mut composer = Composer::new(role);
        composer
            .add(
                az_gem_camera::package_contribution(),
                ProductActivation::default(),
            )
            .expect("camera declares no capability floor");

        let report = composer.finalize().expect("composition is valid");
        assert!(
            report.refusals.is_empty(),
            "camera is unconditional in {role}"
        );
        assert!(
            report.entries.is_empty(),
            "camera registers nothing; {role} saw {:?}",
            report.entries
        );
        assert!(
            report.raw_app_access.is_empty(),
            "camera takes no raw `&mut App` hatch in {role}"
        );
    }
}
