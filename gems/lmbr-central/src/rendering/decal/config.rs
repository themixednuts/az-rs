use bevy::mesh::Mesh;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::projection::DecalProjectionType;
use crate::rendering::{EngineSpec, preview_quad_mesh};

/// Decal configuration.
///
/// Lumberyard reference: `dev/Gems/LmbrCentral/Code/Source/Rendering/DecalComponent.h:36`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors LmbrCentral::DecalConfiguration \
              (47082F75-428F-4353-AC82-FAE8AB017F3B, DecalComponent.h:36) \
              field-for-field; it carries ReflectSerialize/ReflectDeserialize, so these \
              field names are the scene-product encoding, not an internal choice"
)]
pub struct DecalConfiguration {
    pub visible: bool,
    pub projection_type: DecalProjectionType,
    pub material_asset_path: Option<String>,
    pub sort_priority: u32,
    pub depth: f32,
    pub offset: Vec3,
    pub color: Color,
    pub opacity: f32,
    pub deferred: bool,
    pub deferred_post_process: bool,
    pub deferred_string: String,
    pub override_automatic_max_view_distance: bool,
    pub max_view_distance: f32,
    pub automatic_max_view_distance: f32,
    pub view_distance_multiplier: f32,
    pub min_spec: EngineSpec,
}

impl Default for DecalConfiguration {
    fn default() -> Self {
        Self {
            visible: true,
            projection_type: DecalProjectionType::Planar,
            material_asset_path: None,
            sort_priority: 16,
            depth: 1.0,
            offset: Vec3::ZERO,
            color: Color::WHITE,
            opacity: 1.0,
            deferred: false,
            deferred_post_process: false,
            deferred_string: String::new(),
            override_automatic_max_view_distance: false,
            max_view_distance: 8000.0,
            automatic_max_view_distance: 8000.0,
            view_distance_multiplier: 1.0,
            min_spec: EngineSpec::Low,
        }
    }
}

impl DecalConfiguration {
    #[must_use]
    pub const fn is_rendered(&self) -> bool {
        self.visible
    }

    #[must_use]
    pub fn preview_mesh(&self) -> Mesh {
        preview_quad_mesh(self.depth, self.offset)
    }

    #[must_use]
    pub fn preview_material(&self) -> StandardMaterial {
        StandardMaterial {
            base_color: self.color.with_alpha(self.opacity.clamp(0.0, 1.0)),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..Default::default()
        }
    }
}
