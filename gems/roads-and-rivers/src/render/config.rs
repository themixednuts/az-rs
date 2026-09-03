//! Roads and rivers render configuration.

use bevy::prelude::*;

/// Render settings for road and river components.
#[derive(Debug, Clone, Resource, Reflect)]
pub struct RoadsAndRiversRenderConfig {
    pub road_base_color: Color,
    pub river_base_color: Color,
}

impl Default for RoadsAndRiversRenderConfig {
    fn default() -> Self {
        Self {
            road_base_color: Color::srgb(0.23, 0.21, 0.18),
            river_base_color: Color::srgba(0.08, 0.34, 0.55, 0.76),
        }
    }
}

pub(super) fn road_material(config: &RoadsAndRiversRenderConfig) -> StandardMaterial {
    StandardMaterial {
        base_color: config.road_base_color,
        perceptual_roughness: 0.96,
        ..Default::default()
    }
}

pub(super) fn river_material(config: &RoadsAndRiversRenderConfig) -> StandardMaterial {
    StandardMaterial {
        base_color: config.river_base_color,
        alpha_mode: AlphaMode::Blend,
        metallic: 0.0,
        perceptual_roughness: 0.28,
        reflectance: 0.35,
        ..Default::default()
    }
}
