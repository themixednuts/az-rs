//! Fallback vegetation instance rendering data.

use bevy::math::primitives::Cuboid;
use bevy::prelude::*;

pub const DEFAULT_FALLBACK_INSTANCE_SIZE: Vec3 = Vec3::new(0.45, 1.5, 0.45);
pub const DEFAULT_FALLBACK_INSTANCE_ROUGHNESS: f32 = 0.88;

/// Rendering settings for vegetation instances without a resolved scene asset.
#[derive(Debug, Clone, PartialEq, Resource, Reflect)]
pub struct VegetationRenderConfig {
    pub fallback_instance_size: Vec3,
    pub fallback_base_color: Color,
    pub fallback_roughness: f32,
}

impl Default for VegetationRenderConfig {
    fn default() -> Self {
        Self {
            fallback_instance_size: DEFAULT_FALLBACK_INSTANCE_SIZE,
            fallback_base_color: Color::srgb(0.13, 0.38, 0.16),
            fallback_roughness: DEFAULT_FALLBACK_INSTANCE_ROUGHNESS,
        }
    }
}

impl VegetationRenderConfig {
    #[must_use]
    pub fn fallback_mesh(&self) -> Cuboid {
        Cuboid::from_size(self.fallback_instance_size.abs())
    }

    #[must_use]
    pub fn fallback_material(&self) -> StandardMaterial {
        StandardMaterial {
            base_color: self.fallback_base_color,
            perceptual_roughness: self.fallback_roughness,
            ..Default::default()
        }
    }
}

/// Marks an instance rendered with the shared fallback vegetation mesh.
#[derive(Component, Debug, Clone, Copy, Default, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct VegetationFallbackRender;

#[derive(Resource, Debug, Clone, Default)]
pub(super) struct VegetationFallbackRenderAssets {
    mesh: Option<Handle<Mesh>>,
    material: Option<Handle<StandardMaterial>>,
}

impl VegetationFallbackRenderAssets {
    pub(super) fn mesh(
        &mut self,
        meshes: &mut Assets<Mesh>,
        config: &VegetationRenderConfig,
    ) -> Handle<Mesh> {
        if let Some(handle) = &self.mesh {
            return handle.clone();
        }

        let handle = meshes.add(config.fallback_mesh());
        self.mesh = Some(handle.clone());
        handle
    }

    pub(super) fn material(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        config: &VegetationRenderConfig,
    ) -> Handle<StandardMaterial> {
        if let Some(handle) = &self.material {
            return handle.clone();
        }

        let handle = materials.add(config.fallback_material());
        self.material = Some(handle.clone());
        handle
    }
}
