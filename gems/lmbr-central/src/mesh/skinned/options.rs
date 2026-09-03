use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Skinned mesh render options.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/SkinnedMeshComponent.cpp:66`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors LmbrCentral::SkinnedRenderOptions (SkinnedMeshComponent.cpp:66) \
              field-for-field; it carries ReflectSerialize/ReflectDeserialize, so these \
              field names are the scene-product encoding, not an internal choice"
)]
pub struct SkinnedRenderOptions {
    pub opacity: f32,
    pub max_view_distance: f32,
    pub view_distance_multiplier: f32,
    pub lod_ratio: u32,
    pub cast_dynamic_shadows: bool,
    pub use_vis_areas: bool,
    pub rain_occluder: bool,
    pub accept_decals: bool,
}

impl Default for SkinnedRenderOptions {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            max_view_distance: f32::MAX,
            view_distance_multiplier: 1.0,
            lod_ratio: 100,
            cast_dynamic_shadows: true,
            use_vis_areas: true,
            rain_occluder: true,
            accept_decals: true,
        }
    }
}
