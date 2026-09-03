use bevy::prelude::*;

use super::config::FogVolumeConfiguration;

/// Runtime fog volume component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/FogVolumeComponent.h:18`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component)]
pub struct FogVolumeComponent {
    pub configuration: FogVolumeConfiguration,
}
