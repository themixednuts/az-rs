use az_core::EntityId;
use az_derive::AzTypeInfo;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::non_empty_path;
use crate::rendering::preview_quad_mesh;

use super::PARTICLE_EMITTER_SETTINGS_TYPE_UUID;

/// Runtime particle emitter settings.
#[derive(AzTypeInfo, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[az_type_info(name = "ParticleEmitterSettings", PARTICLE_EMITTER_SETTINGS_TYPE_UUID)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the serde(rename) names ARE the ObjectStream wire field names of \
              ParticleEmitterSettings (A1E34557-30DB-4716-B4CE-39D52A113D0C) - \
              Visible, Enable, AttachToMesh, AttachToDissolvingEdge, Pre-roll; \
              packing them stops decoding"
)]
pub struct ParticleEmitterSettings {
    #[serde(rename = "Visible", default)]
    pub visible: bool,
    #[serde(rename = "Enable", default)]
    pub enable: bool,
    #[serde(rename = "AttachToMesh", default)]
    pub attach_to_mesh: bool,
    #[serde(rename = "AttachToDissolvingEdge", default)]
    pub attach_to_dissolving_edge: bool,
    #[serde(rename = "SelectedEmitter", default)]
    pub selected_emitter: String,
    #[serde(rename = "Color", default)]
    pub color: LinearRgba,
    #[serde(rename = "Alpha Scale", default)]
    pub alpha_scale: f32,
    #[serde(rename = "Pre-roll", default)]
    pub pre_roll: bool,
    #[serde(rename = "Particle Count Scale", default)]
    pub particle_count_scale: f32,
    #[serde(rename = "Time Scale", default)]
    pub time_scale: f32,
    #[serde(rename = "Pulse Period", default)]
    pub pulse_period: f32,
    #[serde(rename = "GlobalSizeScale", default)]
    pub global_size_scale: f32,
    #[serde(rename = "ParticleSizeX", default)]
    pub particle_size_x: f32,
    #[serde(rename = "ParticleSizeY", default)]
    pub particle_size_y: f32,
    #[serde(rename = "ParticleSizeZ", default)]
    pub particle_size_z: f32,
    #[serde(rename = "ParticleSizeRandom", default)]
    pub particle_size_random: f32,
    #[serde(rename = "Speed Scale", default)]
    pub speed_scale: f32,
    #[serde(rename = "Strength", default)]
    pub strength: f32,
    #[serde(rename = "Ignore Rotation", default)]
    pub ignore_rotation: bool,
    #[serde(rename = "Not Attached", default)]
    pub not_attached: bool,
    #[serde(rename = "Register by Bounding Box", default)]
    pub register_by_bounding_box: bool,
    #[serde(rename = "Use LOD", default)]
    pub use_lod: bool,
    #[serde(rename = "Target Entity", default)]
    pub target_entity: EntityId,
    #[serde(rename = "GPU Edge Dissolve Target Entity", default)]
    pub gpu_edge_dissolve_target_entity: EntityId,
    #[serde(rename = "Enable Audio", default)]
    pub enable_audio: bool,
    #[serde(rename = "Audio RTPC", default)]
    pub audio_rtpc: String,
    #[serde(rename = "View Distance Multiplier", default)]
    pub view_distance_multiplier: f32,
    #[serde(rename = "Use VisArea", default)]
    pub use_vis_area: bool,
    #[serde(rename = "Accept Decals", default)]
    pub accept_decals: bool,
    #[serde(rename = "Accept Snow", default)]
    pub accept_snow: bool,
    #[serde(rename = "Accept Silhouette", default)]
    pub accept_silhouette: bool,
    #[serde(rename = "Render Always", default)]
    pub render_always: bool,
    #[serde(rename = "Kill On Deactivate", default)]
    pub kill_on_deactivate: bool,
    #[serde(rename = "Force Highest Contextual Priority", default)]
    pub force_highest_contextual_priority: bool,
}

impl Default for ParticleEmitterSettings {
    fn default() -> Self {
        Self {
            visible: true,
            enable: true,
            attach_to_mesh: false,
            attach_to_dissolving_edge: false,
            selected_emitter: String::new(),
            color: LinearRgba::WHITE,
            alpha_scale: 1.0,
            pre_roll: false,
            particle_count_scale: 1.0,
            time_scale: 1.0,
            pulse_period: 0.0,
            global_size_scale: 1.0,
            particle_size_x: 1.0,
            particle_size_y: 1.0,
            particle_size_z: 1.0,
            particle_size_random: 0.0,
            speed_scale: 1.0,
            strength: -1.0,
            ignore_rotation: false,
            not_attached: false,
            register_by_bounding_box: false,
            use_lod: true,
            target_entity: EntityId::default(),
            gpu_edge_dissolve_target_entity: EntityId::default(),
            enable_audio: false,
            audio_rtpc: String::new(),
            view_distance_multiplier: 1.0,
            use_vis_area: true,
            accept_decals: true,
            accept_snow: true,
            accept_silhouette: true,
            render_always: false,
            kill_on_deactivate: false,
            force_highest_contextual_priority: false,
        }
    }
}

impl ParticleEmitterSettings {
    pub const SERIALIZE_VERSION: u32 = 7;

    #[must_use]
    pub fn is_rendered(&self) -> bool {
        self.visible
            && self.enable
            && non_empty_path(Some(self.selected_emitter.as_str())).is_some()
    }

    #[must_use]
    pub fn preview_size(&self) -> f32 {
        let max_axis = self
            .particle_size_x
            .max(self.particle_size_y)
            .max(self.particle_size_z);
        (self.global_size_scale * max_axis).max(0.01)
    }

    #[inline]
    #[must_use]
    pub const fn particle_size(&self) -> Vec3 {
        Vec3::new(
            self.particle_size_x,
            self.particle_size_y,
            self.particle_size_z,
        )
    }

    #[inline]
    pub fn set_particle_size(&mut self, size: impl Into<Vec3>) {
        let size = size.into();
        self.particle_size_x = size.x;
        self.particle_size_y = size.y;
        self.particle_size_z = size.z;
    }

    #[must_use]
    pub fn preview_mesh(&self) -> Mesh {
        preview_quad_mesh(self.preview_size(), Vec3::ZERO)
    }

    #[must_use]
    pub fn preview_material(&self) -> StandardMaterial {
        let alpha = (self.color.alpha * self.alpha_scale).clamp(0.0, 1.0);
        let color = Color::from(self.color).with_alpha(alpha);
        StandardMaterial {
            base_color: color,
            emissive: color.to_linear(),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..Default::default()
        }
    }
}
