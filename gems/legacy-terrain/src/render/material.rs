use bevy::prelude::*;

use super::config::TerrainWaterRenderConfig;

#[must_use]
pub fn terrain_water_material(config: &TerrainWaterRenderConfig) -> StandardMaterial {
    StandardMaterial {
        base_color: config.base_color,
        alpha_mode: AlphaMode::Blend,
        metallic: 0.0,
        perceptual_roughness: 0.24,
        reflectance: 0.42,
        ..Default::default()
    }
}
