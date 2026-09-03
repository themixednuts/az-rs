use bevy::prelude::*;

use crate::{
    ConstantGradientComponent, ConstantGradientConfig, GradientSampleParams, GradientSampler,
    GradientTransformComponent, GradientTransformConfig, InvertGradientComponent,
    InvertGradientConfig, LevelsGradientComponent, LevelsGradientConfig, PerlinGradientComponent,
    PerlinGradientConfig, RandomGradientComponent, RandomGradientConfig,
    ThresholdGradientComponent, ThresholdGradientConfig, TransformType, WrappingType,
};

pub struct GradientSignalPlugin;

impl Plugin for GradientSignalPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<GradientSampleParams>()
            .register_type::<GradientSampler>()
            .register_type::<WrappingType>()
            .register_type::<TransformType>()
            .register_type::<GradientTransformConfig>()
            .register_type::<GradientTransformComponent>()
            .register_type::<ConstantGradientConfig>()
            .register_type::<ConstantGradientComponent>()
            .register_type::<ThresholdGradientConfig>()
            .register_type::<ThresholdGradientComponent>()
            .register_type::<InvertGradientConfig>()
            .register_type::<InvertGradientComponent>()
            .register_type::<LevelsGradientConfig>()
            .register_type::<LevelsGradientComponent>()
            .register_type::<PerlinGradientConfig>()
            .register_type::<PerlinGradientComponent>()
            .register_type::<RandomGradientConfig>()
            .register_type::<RandomGradientComponent>();
    }
}
