use bevy::mesh::Mesh;
use bevy::prelude::*;

use crate::rendering::{EngineSpec, preview_quad_mesh};

/// Runtime lens flare configuration.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/LensFlareComponent.h:34`.
#[derive(Debug, Clone, PartialEq, Reflect)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the reflected property set of LmbrCentral::LensFlareConfiguration \
              (1E28DADD-0BD4-4AD5-A94B-2665813BF346, LensFlareComponent.h:34), whose \
              per-field identity src/tests.rs pins by UUID. Weakest of this group: it \
              has no serde derive yet, so the shape it must match is the vendor \
              property set the ObjectStream reader will address by name, not a format \
              this crate writes today"
)]
pub struct LensFlareConfiguration {
    pub visible: bool,
    pub on_initially: bool,
    pub lens_flare: Option<String>,
    pub min_spec: EngineSpec,
    pub frustum_angle_degrees: f32,
    pub size: f32,
    pub attach_to_sun: bool,
    pub affects_this_area_only: bool,
    pub ignore_vis_areas: bool,
    pub indoor_only: bool,
    pub view_distance_multiplier: f32,
    pub tint: Color,
    pub brightness: f32,
    pub sync_anim_with_light: bool,
    pub light_entity: Option<Entity>,
    pub anim_index: u32,
    pub anim_speed: f32,
    pub anim_phase: f32,
    pub synced_anim_index: u32,
    pub synced_anim_speed: f32,
    pub synced_anim_phase: f32,
}

impl Default for LensFlareConfiguration {
    fn default() -> Self {
        Self {
            visible: true,
            on_initially: true,
            lens_flare: None,
            min_spec: EngineSpec::Low,
            frustum_angle_degrees: 360.0,
            size: 1.0,
            attach_to_sun: false,
            affects_this_area_only: true,
            ignore_vis_areas: false,
            indoor_only: false,
            view_distance_multiplier: 1.0,
            tint: Color::WHITE,
            brightness: 1.0,
            sync_anim_with_light: false,
            light_entity: None,
            anim_index: 0,
            anim_speed: 1.0,
            anim_phase: 0.0,
            synced_anim_index: 0,
            synced_anim_speed: 1.0,
            synced_anim_phase: 0.0,
        }
    }
}

impl LensFlareConfiguration {
    #[must_use]
    pub const fn is_rendered(&self) -> bool {
        self.visible && self.on_initially
    }

    #[must_use]
    pub fn preview_mesh(&self) -> Mesh {
        preview_quad_mesh(self.size, Vec3::ZERO)
    }

    #[must_use]
    pub fn preview_material(&self) -> StandardMaterial {
        let tint = self.tint.to_linear();
        let brightness = self.brightness.max(0.0);
        StandardMaterial {
            base_color: self.tint,
            emissive: tint * brightness,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..Default::default()
        }
    }
}
