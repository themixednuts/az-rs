use bevy::ecs::system::SystemState;
use bevy::prelude::*;

use az_gem_lmbr_central::{BoxShapeComponent, BoxShapeConfig};

use super::*;

#[test]
fn current_gradient_type_ids_match_registry() {
    assert_eq!(
        VEGETATION_GRADIENT_SAMPLER_TYPE_ID,
        uuid::uuid!("00BD6356-F371-475F-A2E2-0A0C3638BD86")
    );
    assert_eq!(
        VEGETATION_GRADIENT_TRANSFORM_COMPONENT_TYPE_ID,
        uuid::uuid!("9CB66205-301C-430B-8339-957534CAEFDF")
    );
    assert_eq!(
        VEGETATION_PERLIN_GRADIENT_COMPONENT_TYPE_ID,
        uuid::uuid!("8C95DACD-84CC-42C5-A49E-5E7A94DBA0EE")
    );
    assert_eq!(
        VEGETATION_PERLIN_GRADIENT_CONFIG_TYPE_ID,
        uuid::uuid!("AC02AF00-B9F2-46D1-9EAC-7DB918269B81")
    );
    assert_eq!(
        VEGETATION_RANDOM_GRADIENT_CONFIG_TYPE_ID,
        uuid::uuid!("705D70DA-EF33-4CE3-903E-3B61C9C3B085")
    );
    assert_eq!(
        VEGETATION_RANDOM_GRADIENT_COMPONENT_TYPE_ID,
        uuid::uuid!("2DFECFD9-7623-49AC-9BCA-704972E6B24B")
    );
}

#[test]
// `float_cmp`: The assertion is that `Default` hands back these exact constants; an epsilon
// would let a wrong default within a tolerance pass.
#[allow(clippy::float_cmp)]
fn sampler_defaults_follow_lumberyard_source() {
    let sampler = GradientSampler::default();

    assert_eq!(sampler.opacity, 1.0);
    assert!(!sampler.invert_input);
    assert!(!sampler.enable_transform);
    assert_eq!(sampler.translate, Vec3::ZERO);
    assert_eq!(sampler.scale, Vec3::ONE);
    assert_eq!(sampler.rotate, Vec3::ZERO);
    assert!(!sampler.enable_levels);
    assert_eq!(sampler.input_mid, 1.0);
    assert_eq!(sampler.input_min, 0.0);
    assert_eq!(sampler.input_max, 1.0);
    assert_eq!(sampler.output_min, 0.0);
    assert_eq!(sampler.output_max, 1.0);
}

#[test]
// `float_cmp`: These pin `levels_value` against the C++ source's own edge-case results; the
// exact value is the property under test.
#[allow(clippy::float_cmp)]
fn levels_value_matches_source_edge_cases() {
    assert_eq!(levels_value(0.25, 1.0, 0.0, 1.0, 0.0, 1.0), 0.25);
    assert_eq!(levels_value(0.25, 1.0, 0.5, 0.5, 0.0, 1.0), 0.0);
    assert_eq!(levels_value(0.75, 1.0, 0.5, 0.5, 0.0, 1.0), 1.0);
    assert_eq!(levels_value(-1.0, 0.0, -1.0, 2.0, 0.25, 0.75), 0.25);
}

#[test]
// `float_cmp`: 0.375 is the exact result of invert-then-opacity on 0.25; a tolerance would
// stop distinguishing that pipeline from a slightly different one.
#[allow(clippy::float_cmp)]
fn sampler_applies_transform_levels_invert_and_opacity() {
    let owner = Entity::from_bits(1);
    let gradient = Entity::from_bits(2);
    let sampler = GradientSampler {
        gradient: Some(gradient),
        owner_entity: Some(owner),
        opacity: 0.5,
        invert_input: true,
        enable_transform: true,
        translate: Vec3::new(1.0, 2.0, 3.0),
        scale: Vec3::splat(2.0),
        rotate: Vec3::ZERO,
        enable_levels: true,
        input_mid: 1.0,
        input_min: 0.0,
        input_max: 1.0,
        output_min: 0.0,
        output_max: 1.0,
    };

    let params = sampler.transform_params(GradientSampleParams {
        position: Vec3::new(1.0, 1.0, 1.0),
    });

    assert_eq!(params.position, Vec3::new(3.0, 4.0, 5.0));
    assert_eq!(sampler.apply_embedded_operations(0.25), 0.375);
}

#[test]
// `float_cmp`: The chain's exact output is the assertion: 0.25 through threshold, invert and
// opacity is 0.5 with no rounding anywhere.
#[allow(clippy::float_cmp)]
fn source_query_samples_entity_backed_gradient_chain() {
    let mut world = World::new();
    let constant = world
        .spawn(ConstantGradientComponent {
            configuration: ConstantGradientConfig { value: 0.25 },
        })
        .id();
    let threshold = world
        .spawn(ThresholdGradientComponent {
            configuration: ThresholdGradientConfig {
                gradient: GradientSampler {
                    gradient: Some(constant),
                    ..Default::default()
                },
                threshold: 0.5,
            },
        })
        .id();
    let sampler = GradientSampler {
        gradient: Some(threshold),
        invert_input: true,
        opacity: 0.5,
        ..Default::default()
    };

    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state
        .get(&world)
        .expect("the source query validates against this world");

    assert_eq!(
        gradients.sample_gradient(&sampler, GradientSampleParams::default()),
        0.5
    );
}

#[test]
// `float_cmp`: A blocked chain must return the literal 0.0 the cycle guard writes, not merely
// something near zero.
#[allow(clippy::float_cmp)]
fn source_query_stops_cyclic_gradient_chains() {
    let mut world = World::new();
    let cyclic = world.spawn_empty().id();
    world.entity_mut(cyclic).insert(ThresholdGradientComponent {
        configuration: ThresholdGradientConfig {
            gradient: GradientSampler {
                gradient: Some(cyclic),
                invert_input: true,
                ..Default::default()
            },
            threshold: 0.5,
        },
    });
    let sampler = GradientSampler {
        gradient: Some(cyclic),
        ..Default::default()
    };

    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state
        .get(&world)
        .expect("the source query validates against this world");

    assert_eq!(
        gradients.sample_gradient(&sampler, GradientSampleParams::default()),
        0.0
    );
}

#[test]
// `float_cmp`: 0.5 is the exact centre value of the deterministic Perlin generator at the
// origin; an epsilon would stop pinning the result.
#[allow(clippy::float_cmp)]
fn source_query_samples_perlin_gradient() {
    let mut world = World::new();
    let perlin = world
        .spawn(PerlinGradientComponent::new(PerlinGradientConfig::default()))
        .id();
    let sampler = GradientSampler {
        gradient: Some(perlin),
        ..Default::default()
    };

    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state
        .get(&world)
        .expect("the source query validates against this world");

    assert_eq!(
        gradients.sample_gradient(&sampler, GradientSampleParams::default()),
        0.5
    );
}

#[test]
// `float_cmp`: The query must reach the identical computation as `config.sample_value`, so
// the two sides have to agree bit for bit.
#[allow(clippy::float_cmp)]
fn source_query_samples_random_gradient() {
    let mut world = World::new();
    let config = RandomGradientConfig {
        random_seed: 5656,
        gradient_scale: 1,
    };
    let random = world.spawn(RandomGradientComponent::new(config)).id();
    let sampler = GradientSampler {
        gradient: Some(random),
        ..Default::default()
    };
    let sample_params = GradientSampleParams {
        position: Vec3::new(2.0, 1.0, 0.0),
    };

    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state
        .get(&world)
        .expect("the source query validates against this world");

    assert_eq!(
        gradients.sample_gradient(&sampler, sample_params),
        config.sample_value(sample_params)
    );
}

#[test]
fn random_gradient_matches_lumberyard_golden_master() {
    let config = RandomGradientConfig {
        random_seed: 5656,
        gradient_scale: 1,
    };
    let expected = [
        0.5059, 0.4902, 0.6000, 0.7372, 0.9490, 0.2823, 0.6588, 0.5804, 0.1490, 0.3294, 0.1451,
        0.6627, 0.2980, 0.1608, 0.9098, 0.9804,
    ];

    for y in 0..4u8 {
        for x in 0..4u8 {
            let actual = config.sample_value(GradientSampleParams {
                position: Vec3::new(f32::from(x), f32::from(y), 0.0),
            });
            let expected = expected[usize::from(y) * 4 + usize::from(x)];
            assert!((actual - expected).abs() <= 0.01);
        }
    }
}

#[test]
fn gradient_transform_converts_position_to_uvw() {
    let config = GradientTransformConfig {
        advanced_mode: true,
        override_bounds: true,
        bounds: Vec3::splat(2.0),
        frequency_zoom: 2.0,
        wrapping_type: WrappingType::None,
        is_3d: true,
        ..Default::default()
    };

    let transformed = config.transform_position_to_uvw(
        Vec3::new(1.25, 0.0, 0.5),
        Transform::from_translation(Vec3::X),
        false,
    );

    assert!(!transformed.rejected);
    assert!(transformed.uvw.abs_diff_eq(Vec3::new(0.5, 0.0, 1.0), 1e-6));
}

#[test]
fn gradient_transform_clamp_to_zero_rejects_max_edge() {
    let config = GradientTransformConfig {
        advanced_mode: true,
        override_bounds: true,
        bounds: Vec3::splat(2.0),
        wrapping_type: WrappingType::ClampToZero,
        is_3d: true,
        ..Default::default()
    };

    let transformed =
        config.transform_position_to_uvw(Vec3::new(1.0, 0.0, 0.0), Transform::IDENTITY, false);

    assert!(transformed.rejected);
    assert!(
        transformed
            .uvw
            .abs_diff_eq(Vec3::new(0.999, 0.0, 0.0), 1e-6)
    );
}

#[test]
fn gradient_transform_config_enums_map_native_values() {
    assert_eq!(
        TransformType::from_native_value(0),
        Some(TransformType::WorldThisEntity)
    );
    assert_eq!(
        TransformType::from_native_value(3),
        Some(TransformType::LocalReferenceEntity)
    );
    assert_eq!(
        TransformType::from_native_value(5),
        Some(TransformType::Relative)
    );
    assert_eq!(TransformType::from_native_value(6), None);
    assert_eq!(TransformType::WorldOrigin.native_value(), 4);

    assert_eq!(WrappingType::from_native_value(0), Some(WrappingType::None));
    assert_eq!(
        WrappingType::from_native_value(4),
        Some(WrappingType::ClampToZero)
    );
    assert_eq!(WrappingType::from_native_value(5), None);
    assert_eq!(WrappingType::Repeat.native_value(), 3);
}

#[test]
fn gradient_transform_uses_configured_output_normalization() {
    let config = GradientTransformConfig {
        advanced_mode: true,
        override_bounds: true,
        bounds: Vec3::splat(4.0),
        normalize_output: true,
        is_3d: true,
        ..Default::default()
    };

    let transformed =
        config.transform_position_to_uvw(Vec3::new(1.0, 0.0, -1.0), Transform::IDENTITY, false);

    assert!(
        transformed
            .uvw
            .abs_diff_eq(Vec3::new(0.75, 0.5, 0.25), 1e-6)
    );
}

#[test]
// `float_cmp`: The transformed query must land on the identical Perlin sample, so the two
// sides have to agree bit for bit.
#[allow(clippy::float_cmp)]
fn source_query_applies_perlin_gradient_transform() {
    let mut world = World::new();
    let perlin_config = PerlinGradientConfig::default();
    let transform_config = GradientTransformConfig {
        advanced_mode: true,
        override_bounds: true,
        bounds: Vec3::splat(2.0),
        override_translate: true,
        translate: Vec3::X,
        frequency_zoom: 2.0,
        is_3d: true,
        ..Default::default()
    };
    let perlin = world
        .spawn((
            PerlinGradientComponent::new(perlin_config),
            GradientTransformComponent {
                configuration: transform_config,
            },
        ))
        .id();
    let sampler = GradientSampler {
        gradient: Some(perlin),
        ..Default::default()
    };

    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state
        .get(&world)
        .expect("the source query validates against this world");
    let sample_params = GradientSampleParams {
        position: Vec3::new(1.25, 0.0, 0.0),
    };

    assert_eq!(
        gradients.sample_gradient(&sampler, sample_params),
        perlin_config.sample_value(GradientSampleParams {
            position: Vec3::new(0.5, 0.0, 0.0),
        })
    );
}

#[test]
// `float_cmp`: The shape-bounds path must reach the identical Perlin sample, so the two sides
// have to agree bit for bit.
#[allow(clippy::float_cmp)]
fn source_query_uses_shape_bounds_for_gradient_transform() {
    let mut world = World::new();
    let perlin_config = PerlinGradientConfig::default();
    let transform_config = GradientTransformConfig {
        advanced_mode: true,
        override_bounds: false,
        bounds: Vec3::ONE,
        wrapping_type: WrappingType::ClampToZero,
        is_3d: true,
        ..Default::default()
    };
    let perlin = world
        .spawn((
            PerlinGradientComponent::new(perlin_config),
            GradientTransformComponent {
                configuration: transform_config,
            },
            BoxShapeComponent {
                configuration: BoxShapeConfig {
                    dimensions: Vec3::splat(4.0),
                },
                ..Default::default()
            },
        ))
        .id();
    let sampler = GradientSampler {
        gradient: Some(perlin),
        ..Default::default()
    };
    let sample_params = GradientSampleParams {
        position: Vec3::new(0.75, 0.0, 0.0),
    };

    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state
        .get(&world)
        .expect("the source query validates against this world");
    let expected = perlin_config.sample_value(sample_params);

    assert!(expected > 0.0);
    assert_eq!(gradients.sample_gradient(&sampler, sample_params), expected);
}

#[test]
// `float_cmp`: These pin `Default` constants and deterministic generator outputs; a
// tolerance would let a wrong default or re-rounded implementation pass.
#[allow(clippy::float_cmp)]
fn component_configs_match_default_values() {
    assert_eq!(ConstantGradientConfig::default().value, 1.0);
    assert_eq!(ThresholdGradientConfig::default().threshold, 0.5);
    assert_eq!(InvertGradientConfig::default().apply_invert(1.25), 0.0);
    assert_eq!(LevelsGradientConfig::default().apply_levels(0.75), 0.75);

    let perlin = PerlinGradientConfig::default();
    assert_eq!(perlin.random_seed, 1);
    assert_eq!(perlin.octave, 1);
    assert_eq!(perlin.amplitude, 1.0);
    assert_eq!(perlin.frequency, 1.0);
    assert_eq!(perlin.sample_value(GradientSampleParams::default()), 0.5);

    let random = RandomGradientConfig::default();
    assert_eq!(random.random_seed, 13);
    assert_eq!(random.gradient_scale, 1);
    assert_eq!(random.normalized_random_seed(), 15);
    assert_eq!(random.effective_gradient_scale(), 1.0);

    let negative_seed = RandomGradientConfig {
        random_seed: -1,
        ..Default::default()
    };
    assert_eq!(
        negative_seed.normalized_random_seed(),
        u64::from(u32::MAX) + 2
    );
}

#[test]
// `float_cmp`: The assertion is that `Default` hands back this exact constant.
#[allow(clippy::float_cmp)]
fn gradient_transform_defaults_match_source() {
    let config = GradientTransformConfig::default();

    assert!(!config.advanced_mode);
    assert!(!config.allow_reference);
    assert_eq!(config.shape_reference, None);
    assert!(!config.override_bounds);
    assert_eq!(config.bounds, Vec3::ONE);
    assert_eq!(config.transform_type, TransformType::WorldThisEntity);
    assert_eq!(config.translate, Vec3::ZERO);
    assert_eq!(config.scale, Vec3::ONE);
    assert_eq!(config.frequency_zoom, 1.0);
    assert!(!config.adjust_frequency_to_bounds);
    assert_eq!(config.wrapping_type, WrappingType::None);
    assert!(!config.normalize_output);
    assert!(!config.is_3d);
}
