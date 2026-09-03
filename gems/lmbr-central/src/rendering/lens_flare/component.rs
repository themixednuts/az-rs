use bevy::prelude::*;

use super::config::LensFlareConfiguration;

/// Runtime lens flare component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/LensFlareComponent.h:102`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component)]
pub struct LensFlareComponent {
    pub configuration: LensFlareConfiguration,
}
