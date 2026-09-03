use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Static mesh render options.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/MeshComponent.cpp:168`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors LmbrCentral::MeshRenderOptions (MeshComponent.cpp:168) \
              field-for-field; it carries ReflectSerialize/ReflectDeserialize, so these \
              field names are the scene-product encoding, not an internal choice"
)]
pub struct MeshRenderOptions {
    pub opacity: f32,
    pub cross_fade_time: f32,
    pub max_view_distance: f32,
    pub editor_computed_view_distance: f32,
    pub view_distance_multiplier: f32,
    pub lod_ratio: u32,
    pub use_vis_areas: bool,
    pub cast_shadows: bool,
    pub lod_bounding_box_based: bool,
    pub rain_occluder: bool,
    pub affect_navmesh: bool,
    pub affect_dynamic_water: bool,
    pub receive_wind_based_on_material: bool,
    pub accept_decals: bool,
    pub accept_snow: bool,
    pub accept_sand: bool,
    pub accept_silhouette: bool,
    pub receive_wind: bool,
    pub wind_bend_scale: f32,
    pub visibility_occluder: bool,
    pub always_render: bool,
    pub lod_min_screen_pct: Vec<f32>,
    pub sort_type: u8,
    pub should_instance: bool,
    pub should_merge: bool,
    pub force_merge: bool,
    pub fade_enabled: bool,
    pub primary_in_hierarchy: bool,
    pub use_manual_view_distance: bool,
    pub extended_camera_planes: bool,
    pub dynamic_mesh: bool,
    pub has_static_transform: bool,
    pub affect_gi: bool,
}

impl Default for MeshRenderOptions {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            cross_fade_time: 0.0,
            max_view_distance: f32::MAX,
            editor_computed_view_distance: 0.0,
            view_distance_multiplier: 1.0,
            lod_ratio: 100,
            use_vis_areas: true,
            cast_shadows: true,
            lod_bounding_box_based: false,
            rain_occluder: true,
            affect_navmesh: true,
            affect_dynamic_water: false,
            receive_wind_based_on_material: false,
            accept_decals: true,
            accept_snow: false,
            accept_sand: false,
            accept_silhouette: false,
            receive_wind: false,
            wind_bend_scale: 0.0,
            visibility_occluder: false,
            always_render: false,
            lod_min_screen_pct: Vec::new(),
            sort_type: 0,
            should_instance: false,
            should_merge: false,
            force_merge: false,
            fade_enabled: false,
            primary_in_hierarchy: false,
            use_manual_view_distance: false,
            extended_camera_planes: false,
            dynamic_mesh: false,
            has_static_transform: false,
            affect_gi: true,
        }
    }
}

impl MeshRenderOptions {
    #[must_use]
    pub const fn is_static(&self) -> bool {
        self.has_static_transform && !self.dynamic_mesh && !self.receive_wind
    }

    #[must_use]
    pub const fn affects_gi(&self) -> bool {
        self.affect_gi && self.is_static()
    }
}
