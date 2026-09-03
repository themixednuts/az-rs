use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::kinds::{LightCubemapResolution, LightType, VoxelGiMode};
use crate::rendering::EngineSpec;

/// Runtime light configuration.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/LightComponent.h:39`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors LmbrCentral::LightConfiguration \
              (F4CC7BB4-C541-480C-88FC-C5A8F37CC67F, LightComponent.h:39) \
              field-for-field; it carries ReflectSerialize/ReflectDeserialize, so these \
              field names are the scene-product encoding, not an internal choice"
)]
pub struct LightConfiguration {
    pub light_type: LightType,
    pub on_initially: bool,
    pub visible: bool,
    pub point_max_distance: f32,
    pub point_attenuation_bulb_size: f32,
    pub area_width: f32,
    pub area_height: f32,
    pub area_max_distance: f32,
    pub area_fov_degrees: f32,
    pub projector_range: f32,
    pub projector_attenuation_bulb_size: f32,
    pub projector_fov_degrees: f32,
    pub projector_near_plane: f32,
    pub projector_texture_asset_path: Option<String>,
    pub projector_material_asset_path: Option<String>,
    pub probe_area: Vec3,
    pub probe_sort_priority: u32,
    pub probe_cubemap_resolution: LightCubemapResolution,
    pub probe_cubemap_asset_path: Option<String>,
    pub box_projected: bool,
    pub box_width: f32,
    pub box_height: f32,
    pub box_length: f32,
    pub attenuation_falloff_max: f32,
    pub tod_influence: f32,
    pub probe_fade: f32,
    pub min_spec: EngineSpec,
    pub view_distance_cap_enabled: bool,
    pub view_distance_multiplier: f32,
    pub view_distance_cap: f32,
    pub cast_shadows_spec: EngineSpec,
    pub voxel_gi_mode: VoxelGiMode,
    pub color: Color,
    pub diffuse_multiplier: f32,
    pub specular_multiplier: f32,
    pub affects_this_area_only: bool,
    pub use_vis_areas: bool,
    pub indoor_only: bool,
    pub ambient: bool,
    pub deferred: bool,
    pub anim_index: u32,
    pub anim_speed: f32,
    pub anim_phase: f32,
    pub volumetric_fog: bool,
    pub volumetric_fog_only: bool,
    pub cast_terrain_shadows: bool,
    pub shadow_bias: f32,
    pub shadow_slope_bias: f32,
    pub shadow_res_scale: f32,
    pub shadow_update_min_radius: f32,
    pub shadow_update_ratio: f32,
    pub shadow_max_camera_distance: f32,
    pub cubemap_id: Option<String>,
    pub anim_phase_random: bool,
    #[serde(skip, default)]
    #[reflect(ignore)]
    pub clip_volume_entity: Option<Entity>,
}
