//! High-quality shadow component data.

use bevy::prelude::*;

/// Lumberyard `LmbrCentral::HighQualityShadowComponent` AZ component UUID.
pub const HIGH_QUALITY_SHADOW_COMPONENT_TYPE_ID: &str = "B692F9D9-4850-4D6E-9A32-760901455E40";

/// Lumberyard `LmbrCentral::HighQualityShadowConfig` type UUID.
pub const HIGH_QUALITY_SHADOW_CONFIG_TYPE_ID: &str = "3B3CD21A-E61B-401A-8F54-B76FB6278B11";

/// Per-entity shadow-map settings.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/HighQualityShadowComponent.h:24`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct HighQualityShadowConfig {
    pub enabled: bool,
    pub const_bias: f32,
    pub slope_bias: f32,
    pub jitter: f32,
    pub bbox_scale: Vec3,
    pub shadow_map_size: i32,
}

impl Default for HighQualityShadowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            const_bias: 0.001,
            slope_bias: 0.01,
            jitter: 0.01,
            bbox_scale: Vec3::ONE,
            shadow_map_size: 1024,
        }
    }
}

impl HighQualityShadowConfig {
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn shadow_map_size(&self) -> u32 {
        u32::try_from(self.shadow_map_size.max(1)).unwrap_or(1)
    }
}

/// Runtime high-quality shadow component.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/HighQualityShadowComponent.h:48`.
#[derive(Component, Debug, Clone, Default, PartialEq, Reflect)]
#[reflect(Component)]
pub struct HighQualityShadowComponent {
    pub config: HighQualityShadowConfig,
}

pub(super) fn register_high_quality_shadow_components(app: &mut App) {
    app.register_type::<HighQualityShadowConfig>()
        .register_type::<HighQualityShadowComponent>();
}
