use bevy::prelude::*;

use super::*;
use crate::descriptor::{OverrideMode, VegetationDescriptor};
use crate::{
    InstanceData, VegetationSurfaceTag, VegetationSurfaceTagWeight, quat_close, vec3_close,
};

#[test]
// `float_cmp`: The assertion is that `Default` hands back these exact Lumberyard constants.
#[allow(clippy::float_cmp)]
fn filter_and_modifier_defaults_match_source_in_bevy_axes() {
    let distribution = DistributionFilterConfig::default();
    let altitude = SurfaceAltitudeFilterConfig::default();
    let surface_mask = SurfaceMaskFilterConfig::default();
    let surface_slope = SurfaceSlopeFilterConfig::default();
    let position = PositionModifierConfig::default();
    let rotation = RotationModifierConfig::default();
    let scale = ScaleModifierConfig::default();
    let slope = SlopeAlignmentModifierConfig::default();

    assert_eq!(distribution.filter_stage, FilterStage::Default);
    assert_eq!(distribution.threshold_min, 0.1);
    assert_eq!(distribution.threshold_max, 1.0);

    assert_eq!(altitude.filter_stage, FilterStage::Default);
    assert!(!altitude.allow_overrides);
    assert_eq!(altitude.altitude_min, 0.0);
    assert_eq!(altitude.altitude_max, 128.0);

    assert_eq!(surface_mask.filter_stage, FilterStage::Default);
    assert!(!surface_mask.allow_overrides);
    assert!(surface_mask.inclusive_surface_masks.is_empty());
    assert_eq!(surface_mask.inclusive_weight_min, 0.1);
    assert_eq!(surface_mask.inclusive_weight_max, 1.0);
    assert!(surface_mask.exclusive_surface_masks.is_empty());
    assert_eq!(surface_mask.exclusive_weight_min, 0.1);
    assert_eq!(surface_mask.exclusive_weight_max, 1.0);

    assert_eq!(surface_slope.filter_stage, FilterStage::Default);
    assert!(!surface_slope.allow_overrides);
    assert_eq!(surface_slope.slope_min_degrees, 0.0);
    assert_eq!(surface_slope.slope_max_degrees, 180.0);

    assert_eq!(position.modifier_stage(), ModifierStage::PreProcess);
    assert!(!position.allow_overrides);
    assert_eq!(position.range_min, Vec3::new(-0.3, 0.0, -0.3));
    assert_eq!(position.range_max, Vec3::new(0.3, 0.0, 0.3));
    assert!(!position.align_to_water);
    assert_eq!(position.water_height_offset, 0.0);

    assert_eq!(rotation.modifier_stage(), ModifierStage::Standard);
    assert_eq!(rotation.range_min_degrees, Vec3::new(0.0, -180.0, 0.0));
    assert_eq!(rotation.range_max_degrees, Vec3::new(0.0, 180.0, 0.0));

    assert_eq!(scale.modifier_stage(), ModifierStage::Standard);
    assert_eq!(scale.range_min, 1.0);
    assert_eq!(scale.range_max, 1.0);

    assert_eq!(slope.modifier_stage(), ModifierStage::Standard);
    assert_eq!(slope.range_min, 1.0);
    assert_eq!(slope.range_max, 1.0);
}

#[test]
fn distribution_filter_accepts_inclusive_threshold_range() {
    let config = DistributionFilterConfig {
        threshold_min: 0.25,
        threshold_max: 0.75,
        ..Default::default()
    };

    assert!(!config.accepts_sample(0.249));
    assert!(config.accepts_sample(0.25));
    assert!(config.accepts_sample(0.75));
    assert!(!config.accepts_sample(0.751));
}

#[test]
fn surface_altitude_filter_uses_bevy_height_axis_and_descriptor_overrides() {
    let config = SurfaceAltitudeFilterConfig {
        altitude_min: 2.0,
        altitude_max: 8.0,
        ..Default::default()
    };
    let low = InstanceData {
        position: Vec3::new(0.0, 1.9, 0.0),
        ..Default::default()
    };
    let inside = InstanceData {
        position: Vec3::new(0.0, 4.0, 0.0),
        ..Default::default()
    };

    assert!(!config.accepts_instance(&low, None));
    assert!(config.accepts_instance(&inside, None));

    let override_descriptor = VegetationDescriptor {
        altitude_filter_override_enabled: true,
        altitude_filter_min: 10.0,
        altitude_filter_max: 12.0,
        ..Default::default()
    };
    let override_config = SurfaceAltitudeFilterConfig {
        allow_overrides: true,
        ..config
    };

    assert!(!override_config.accepts_instance(&inside, Some(&override_descriptor)));
}

#[test]
fn surface_slope_filter_uses_bevy_up_axis_and_descriptor_overrides() {
    let config = SurfaceSlopeFilterConfig {
        slope_min_degrees: 0.0,
        slope_max_degrees: 20.0,
        ..Default::default()
    };
    let flat = InstanceData {
        normal: Vec3::Y,
        ..Default::default()
    };
    let vertical = InstanceData {
        normal: Vec3::Z,
        ..Default::default()
    };

    assert!(config.accepts_instance(&flat, None));
    assert!(!config.accepts_instance(&vertical, None));

    let override_descriptor = VegetationDescriptor {
        slope_filter_override_enabled: true,
        slope_filter_min: 80.0,
        slope_filter_max: 100.0,
        ..Default::default()
    };
    let override_config = SurfaceSlopeFilterConfig {
        allow_overrides: true,
        ..config
    };

    assert!(override_config.accepts_instance(&vertical, Some(&override_descriptor)));
}

#[test]
fn surface_mask_filter_applies_component_and_descriptor_tags() {
    let instance = InstanceData {
        masks: vec![VegetationSurfaceTagWeight::new(
            VegetationSurfaceTag::TERRAIN,
            1.0,
        )],
        ..Default::default()
    };
    let include_terrain = SurfaceMaskFilterConfig {
        inclusive_surface_masks: vec![VegetationSurfaceTag::TERRAIN],
        ..Default::default()
    };

    assert!(include_terrain.accepts_instance(&instance, None));

    let exclude_terrain = SurfaceMaskFilterConfig {
        exclusive_surface_masks: vec![VegetationSurfaceTag::TERRAIN],
        ..Default::default()
    };
    assert!(!exclude_terrain.accepts_instance(&instance, None));

    let descriptor = VegetationDescriptor {
        surface_filter_override_mode: OverrideMode::Replace,
        inclusive_surface_filter_tags: vec![VegetationSurfaceTag::TERRAIN_HOLE],
        ..Default::default()
    };
    let override_filter = SurfaceMaskFilterConfig {
        allow_overrides: true,
        inclusive_surface_masks: vec![VegetationSurfaceTag::TERRAIN],
        ..Default::default()
    };

    assert!(!override_filter.accepts_instance(&instance, Some(&descriptor)));
}

#[test]
// `float_cmp`: Descriptor overrides must produce these exact multipliers and factors.
#[allow(clippy::float_cmp)]
fn modifiers_apply_descriptor_overrides_when_enabled() {
    let descriptor = VegetationDescriptor {
        position_override_enabled: true,
        position_min: Vec3::new(-2.0, 0.0, -4.0),
        position_max: Vec3::new(2.0, 8.0, 4.0),
        rotation_override_enabled: true,
        rotation_min_degrees: Vec3::new(0.0, -90.0, 0.0),
        rotation_max_degrees: Vec3::new(0.0, 90.0, 0.0),
        scale_override_enabled: true,
        scale_min: 0.5,
        scale_max: 2.5,
        surface_alignment_override_enabled: true,
        surface_alignment_min: 0.25,
        surface_alignment_max: 0.75,
        ..Default::default()
    };

    let position = PositionModifierConfig {
        allow_overrides: true,
        ..Default::default()
    };
    assert_eq!(
        position.offset_from_factors(Vec3::new(0.25, 0.5, 1.0), Some(&descriptor)),
        Vec3::new(-1.0, 4.0, 4.0)
    );

    let rotation = RotationModifierConfig {
        allow_overrides: true,
        ..Default::default()
    };
    assert_eq!(
        rotation.euler_degrees_from_factors(Vec3::new(0.0, 0.75, 0.0), Some(&descriptor)),
        Vec3::new(0.0, 45.0, 0.0)
    );

    let scale = ScaleModifierConfig {
        allow_overrides: true,
        ..Default::default()
    };
    assert_eq!(
        scale.scale_multiplier_from_factor(0.25, Some(&descriptor)),
        1.0
    );
    assert_eq!(scale.apply_scale(0.0, 0.25, Some(&descriptor)), 0.01);

    let slope = SlopeAlignmentModifierConfig {
        allow_overrides: true,
        ..Default::default()
    };
    assert_eq!(
        slope.alignment_factor_from_sample(0.5, Some(&descriptor)),
        0.5
    );
}

#[test]
fn rotation_modifier_default_yaws_around_bevy_up_axis() {
    let config = RotationModifierConfig::default();
    let rotation = config.rotation_from_factors(Vec3::new(0.0, 0.75, 0.0), None);

    assert!(quat_close(
        rotation,
        Quat::from_rotation_y(90.0_f32.to_radians())
    ));
}

#[test]
fn slope_alignment_builds_bevy_quaternion_from_surface_normal() {
    let config = SlopeAlignmentModifierConfig::default();

    let flat = config.alignment_from_sample(Vec3::Y, 1.0, None);
    assert!(quat_close(flat, Quat::IDENTITY));

    let normal = Vec3::Z;
    let aligned = SlopeAlignmentModifierConfig::alignment_from_factor(normal, 1.0);
    assert!(vec3_close(aligned.mul_vec3(Vec3::Y), normal));
}
