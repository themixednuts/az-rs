//! Terrain materials.

use bevy::image::Image;
use bevy::prelude::*;

/// Terrain material with multiple layers.
#[derive(Asset, TypePath, Debug, Clone)]
pub struct TerrainMaterial {
    /// Layer 0 (base) albedo texture
    pub layer0_albedo: Handle<Image>,

    /// Layer 1 albedo texture
    pub layer1_albedo: Option<Handle<Image>>,

    /// Layer 2 albedo texture
    pub layer2_albedo: Option<Handle<Image>>,

    /// Layer 3 albedo texture
    pub layer3_albedo: Option<Handle<Image>>,

    /// Blend map (RGBA = layer weights)
    pub blend_map: Option<Handle<Image>>,

    /// Texture scale (tiling)
    pub texture_scale: Vec4, // x, y, z, w for each layer
}

/// Terrain material layer.
#[derive(Debug, Clone, Reflect)]
pub struct TerrainMaterialLayer {
    /// Layer name
    pub name: String,

    /// Albedo (diffuse) texture
    pub albedo_texture: Handle<Image>,

    /// Normal map texture
    pub normal_texture: Option<Handle<Image>>,

    /// Texture tiling scale
    pub texture_scale: f32,

    /// Blending weight (0.0 to 1.0)
    pub weight: f32,
}

impl Default for TerrainMaterialLayer {
    fn default() -> Self {
        Self {
            name: "Untitled".to_string(),
            albedo_texture: Handle::default(),
            normal_texture: None,
            texture_scale: 1.0,
            weight: 1.0,
        }
    }
}

impl TerrainMaterialLayer {
    /// Create a new material layer.
    pub fn new(name: impl Into<String>, albedo: Handle<Image>) -> Self {
        Self {
            name: name.into(),
            albedo_texture: albedo,
            normal_texture: None,
            texture_scale: 1.0,
            weight: 1.0,
        }
    }

    /// Set normal map.
    #[must_use]
    pub fn with_normal(mut self, normal: Handle<Image>) -> Self {
        self.normal_texture = Some(normal);
        self
    }

    /// Set texture scale.
    #[must_use]
    pub const fn with_scale(mut self, scale: f32) -> Self {
        self.texture_scale = scale;
        self
    }

    /// Set blend weight.
    #[must_use]
    pub const fn with_weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }
}
