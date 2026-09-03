use bevy::light::EnvironmentMapLight as BevyEnvironmentMapLight;
use bevy::prelude::*;

use super::core::LightConfiguration;

impl LightConfiguration {
    #[must_use]
    pub const fn is_rendered(&self) -> bool {
        self.visible && self.on_initially
    }

    #[must_use]
    pub fn bevy_intensity(&self) -> f32 {
        self.diffuse_multiplier.max(0.0) * 100_000.0
    }

    #[must_use]
    pub fn point_light(&self) -> PointLight {
        PointLight {
            color: self.color,
            intensity: self.bevy_intensity(),
            range: self.point_max_distance.max(0.0),
            radius: self.point_attenuation_bulb_size.max(0.0),
            shadow_maps_enabled: self.cast_shadows_spec.enables_shadow_casting(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn area_light_as_point_light(&self) -> PointLight {
        PointLight {
            color: self.color,
            intensity: self.bevy_intensity(),
            range: self.area_max_distance.max(0.0),
            radius: self.area_width.max(self.area_height).max(0.0) * 0.5,
            shadow_maps_enabled: self.cast_shadows_spec.enables_shadow_casting(),
            ..Default::default()
        }
    }

    #[must_use]
    pub fn spot_light(&self) -> SpotLight {
        let outer_angle = (self.projector_fov_degrees * 0.5)
            .to_radians()
            .clamp(0.01, core::f32::consts::FRAC_PI_2 - 0.01);
        SpotLight {
            color: self.color,
            intensity: self.bevy_intensity(),
            range: self.projector_range.max(0.0),
            radius: self.projector_attenuation_bulb_size.max(0.0),
            shadow_maps_enabled: self.cast_shadows_spec.enables_shadow_casting(),
            shadow_map_near_z: self.projector_near_plane.max(0.0),
            outer_angle,
            inner_angle: outer_angle * 0.5,
            ..Default::default()
        }
    }

    #[must_use]
    pub fn probe_transform_scale(&self) -> Vec3 {
        self.probe_area.max(Vec3::splat(0.001))
    }

    #[must_use]
    pub const fn environment_map_intensity(&self) -> f32 {
        self.diffuse_multiplier
            .max(self.specular_multiplier)
            .max(0.0)
    }

    #[must_use]
    pub fn environment_map_light(
        &self,
        diffuse_map: Handle<Image>,
        specular_map: Handle<Image>,
    ) -> BevyEnvironmentMapLight {
        BevyEnvironmentMapLight {
            diffuse_map,
            specular_map,
            intensity: self.environment_map_intensity(),
            ..Default::default()
        }
    }
}
