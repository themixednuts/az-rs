//! Shared gradient sampler data.

use bevy::ecs::entity::MapEntities;
use bevy::math::{EulerRot, Mat4, Quat, Vec3};
use bevy::prelude::*;

use super::util::levels_value;

/// Position passed to a gradient source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect)]
pub struct GradientSampleParams {
    pub position: Vec3,
}

/// Shared input gradient sampler.
///
/// O3DE reference: `Gems/GradientSignal/Code/Include/GradientSignal/GradientSampler.h:42`.
#[derive(Debug, Clone, PartialEq, Reflect, MapEntities)]
pub struct GradientSampler {
    #[entities]
    pub gradient: Option<Entity>,
    #[entities]
    pub owner_entity: Option<Entity>,
    pub opacity: f32,
    pub invert_input: bool,
    pub enable_transform: bool,
    pub translate: Vec3,
    pub scale: Vec3,
    pub rotate: Vec3,
    pub enable_levels: bool,
    pub input_mid: f32,
    pub input_min: f32,
    pub input_max: f32,
    pub output_min: f32,
    pub output_max: f32,
}

impl Default for GradientSampler {
    fn default() -> Self {
        Self {
            gradient: None,
            owner_entity: None,
            opacity: 1.0,
            invert_input: false,
            enable_transform: false,
            translate: Vec3::ZERO,
            scale: Vec3::ONE,
            rotate: Vec3::ZERO,
            enable_levels: false,
            input_mid: 1.0,
            input_min: 0.0,
            input_max: 1.0,
            output_min: 0.0,
            output_max: 1.0,
        }
    }
}

impl GradientSampler {
    /// Whether any levels parameter differs from its constructed default.
    // Exact comparison is the question being asked: these five fields are
    // tested against the very constants `Default` assigns them, and an epsilon
    // would read an edited-but-near-default value as untouched and skip the
    // levels remap the editor asked for.
    #[allow(clippy::float_cmp)]
    #[must_use]
    pub fn has_level_params(&self) -> bool {
        self.input_mid != 1.0
            || self.input_min != 0.0
            || self.input_max != 1.0
            || self.output_min != 0.0
            || self.output_max != 1.0
    }

    #[must_use]
    pub fn has_transform_params(&self) -> bool {
        self.translate != Vec3::ZERO || self.rotate != Vec3::ZERO || self.scale != Vec3::ONE
    }

    #[must_use]
    pub fn transform_params(&self, params: GradientSampleParams) -> GradientSampleParams {
        if !self.enable_transform || !self.has_transform_params() {
            return params;
        }

        let rotation = Quat::from_euler(
            EulerRot::XYZ,
            self.rotate.x.to_radians(),
            self.rotate.y.to_radians(),
            self.rotate.z.to_radians(),
        );
        let transform = Mat4::from_scale_rotation_translation(self.scale, rotation, self.translate);

        GradientSampleParams {
            position: transform.transform_point3(params.position),
        }
    }

    #[must_use]
    pub fn apply_embedded_operations(&self, sampled_value: f32) -> f32 {
        if self.opacity <= 0.0 || self.gradient.is_none() {
            return 0.0;
        }

        let mut output = sampled_value;
        if self.invert_input {
            output = 1.0 - output;
        }
        if self.enable_levels && self.has_level_params() {
            output = levels_value(
                output,
                self.input_mid,
                self.input_min,
                self.input_max,
                self.output_min,
                self.output_max,
            );
        }
        output * self.opacity
    }
}
