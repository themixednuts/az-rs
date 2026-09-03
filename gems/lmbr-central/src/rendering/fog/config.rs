use bevy::light::FogVolume as BevyFogVolume;
use bevy::prelude::*;

use super::volume_type::FogVolumeType;
use crate::rendering::EngineSpec;

/// Runtime fog volume configuration.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/FogVolumeCommon.h:28`.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct FogVolumeConfiguration {
    pub min_spec: EngineSpec,
    pub view_distance_multiplier: f32,
    pub volume_type: FogVolumeType,
    pub color: Color,
    pub size: Vec3,
    pub hdr_dynamic: f32,
    pub use_global_fog_color: bool,
    pub global_density: f32,
    pub density_offset: f32,
    pub near_cutoff: f32,
    pub fall_off_dir_long: f32,
    pub fall_off_dir_latitude: f32,
    pub fall_off_shift: f32,
    pub fall_off_scale: f32,
    pub soft_edges: f32,
    pub ramp_start: f32,
    pub ramp_end: f32,
    pub ramp_influence: f32,
    pub wind_influence: f32,
    pub density_noise_scale: f32,
    pub density_noise_offset: f32,
    pub density_noise_time_frequency: f32,
    pub density_noise_frequency: Vec3,
    pub ignores_vis_areas: bool,
    pub affects_this_area_only: bool,
}

impl Default for FogVolumeConfiguration {
    fn default() -> Self {
        Self {
            min_spec: EngineSpec::Low,
            view_distance_multiplier: 1.0,
            volume_type: FogVolumeType::Ellipsoid,
            color: Color::WHITE,
            size: Vec3::ONE,
            hdr_dynamic: 0.0,
            use_global_fog_color: false,
            global_density: 1.0,
            density_offset: 0.0,
            near_cutoff: 0.0,
            fall_off_dir_long: 0.0,
            fall_off_dir_latitude: 90.0,
            fall_off_shift: 0.0,
            fall_off_scale: 1.0,
            soft_edges: 1.0,
            ramp_start: 1.0,
            ramp_end: 50.0,
            ramp_influence: 0.0,
            wind_influence: 1.0,
            density_noise_scale: 1.0,
            density_noise_offset: 1.0,
            density_noise_time_frequency: 0.0,
            density_noise_frequency: Vec3::splat(10.0),
            ignores_vis_areas: false,
            affects_this_area_only: false,
        }
    }
}

impl FogVolumeConfiguration {
    #[must_use]
    pub fn is_rendered(&self) -> bool {
        self.volume_type != FogVolumeType::None
    }

    #[must_use]
    pub fn normalized_size(&self) -> Vec3 {
        self.size.max(Vec3::splat(0.001))
    }

    #[must_use]
    pub fn bevy_fog_volume(&self) -> BevyFogVolume {
        BevyFogVolume {
            fog_color: self.color,
            density_factor: self.global_density.max(0.01),
            light_tint: self.color,
            light_intensity: if self.hdr_dynamic > 0.0 {
                self.hdr_dynamic
            } else {
                1.0
            },
            ..Default::default()
        }
    }
}
