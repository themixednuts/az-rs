use super::*;
use bevy::ecs::system::SystemState;

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn shape_components_defaults_match_lumberyard_runtime_defaults() {
    let box_shape = BoxShapeComponent::default();
    let sphere = SphereShapeComponent::default();
    let capsule = CapsuleShapeComponent::default();
    let cylinder = CylinderShapeComponent::default();
    let compound = CompoundShapeComponent::default();

    assert_eq!(box_shape.configuration.dimensions, Vec3::ONE);
    assert_eq!(box_shape.bevy_cuboid().half_size, Vec3::splat(0.5));
    assert_eq!(box_shape.local_bounds().min, Vec3::splat(-0.5).into());
    assert_eq!(box_shape.local_bounds().max, Vec3::splat(0.5).into());

    assert_eq!(sphere.configuration.radius, 0.5);
    assert_eq!(sphere.bevy_sphere().radius, 0.5);
    assert_eq!(sphere.local_bounds().radius(), 0.5);

    assert_eq!(capsule.configuration.height, 1.0);
    assert_eq!(capsule.configuration.radius, 0.25);
    assert_eq!(capsule.configuration.cylinder_segment_length(), 0.5);
    assert_eq!(capsule.bevy_capsule().radius, 0.25);
    assert_eq!(capsule.bevy_capsule().half_length, 0.25);
    assert_eq!(
        capsule.configuration.local_capsule_points(),
        (Vec3::new(0.0, 0.0, -0.25), Vec3::new(0.0, 0.0, 0.25))
    );

    assert_eq!(cylinder.configuration.height, 1.0);
    assert_eq!(cylinder.configuration.radius, 0.5);
    assert!(!cylinder.configuration.ignore_ends);
    assert_eq!(cylinder.bevy_cylinder().radius, 0.5);
    assert_eq!(cylinder.bevy_cylinder().half_height, 0.5);

    assert!(compound.configuration.is_empty());
    assert_eq!(compound.configuration.child_count(), 0);
}

#[test]
fn spline_components_preserve_native_defaults_and_type_changes() {
    let mut spline = SplineComponent::default();

    assert_eq!(spline.configuration.spline_type(), SplineType::Linear);
    assert!(spline.spline_shape_asset_id.is_nil());
    assert!(spline.configuration.spline.data().is_empty());
    assert_eq!(spline.configuration.spline.data().segment_count(), 0);

    spline.configuration.spline.data_mut().vertices = vec![
        Vec3::new(-1.0, 2.0, 0.0),
        Vec3::new(3.0, -2.0, 4.0),
        Vec3::new(0.0, 1.0, -1.0),
    ];

    let bounds = spline.configuration.spline.local_bounds().unwrap();
    assert_eq!(bounds.min, Vec3::new(-1.0, -2.0, -1.0).into());
    assert_eq!(bounds.max, Vec3::new(3.0, 2.0, 4.0).into());

    spline.configuration.change_spline_type(SplineType::Bezier);
    assert_eq!(spline.configuration.spline_type(), SplineType::Bezier);
    assert_eq!(spline.configuration.spline.data().len(), 3);

    let Spline::Bezier(bezier) = &spline.configuration.spline else {
        panic!("expected Bezier spline");
    };
    assert_eq!(bezier.granularity, 8);
    assert_eq!(bezier.clamped_granularity(), 8);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn polygon_prism_defaults_and_bounds_match_lumberyard_shape_space() {
    let mut component = PolygonPrismShapeComponent::default();

    assert!(component.polygon_shape_asset_id.is_nil());
    assert_eq!(component.configuration.polygon_prism.height, 1.0);
    assert!(component.configuration.polygon_prism.is_empty());
    assert!(component.configuration.local_bounds().is_none());

    component.configuration.polygon_prism.vertices = vec![
        Vec2::new(-2.0, 1.0),
        Vec2::new(3.0, -4.0),
        Vec2::new(1.0, 2.0),
    ];
    component.configuration.polygon_prism.height = 5.0;

    let bounds = component.configuration.local_bounds().unwrap();
    assert_eq!(component.configuration.polygon_prism.vertex_count(), 3);
    assert_eq!(bounds.min, Vec3::new(-2.0, -4.0, 0.0).into());
    assert_eq!(bounds.max, Vec3::new(3.0, 2.0, 5.0).into());
}

#[test]
fn polygon_prism_decomposition_keeps_native_convex_fast_path() {
    let prism = PolygonPrism {
        height: 3.0,
        vertices: vec![
            Vec2::new(-1.0, -1.0),
            Vec2::new(1.0, -1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-1.0, 1.0),
        ],
    };

    let decomposition = prism.decompose().unwrap();
    assert_eq!(decomposition.as_ref(), [prism.vertices]);
}

#[test]
fn polygon_prism_decomposition_triangulates_and_convex_merges_concave_faces() {
    let prism = PolygonPrism {
        height: 2.0,
        vertices: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(3.0, 0.0),
            Vec2::new(3.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 3.0),
            Vec2::new(0.0, 3.0),
        ],
    };

    let decomposition = prism.decompose().unwrap();
    assert!(decomposition.as_ref().len() > 1);
    assert!(
        decomposition
            .as_ref()
            .iter()
            .all(|face| face.len() <= MAX_POLYGON_PRISM_EDGES)
    );
    let decomposed_area = decomposition
        .as_ref()
        .iter()
        .map(|face| {
            face.iter()
                .zip(face.iter().cycle().skip(1))
                .map(|(a, b)| b.x.mul_add(-a.y, a.x * b.y))
                .sum::<f32>()
                .abs()
                * 0.5
        })
        .sum::<f32>();
    assert!((decomposed_area - 5.0).abs() < 1.0e-5);
}

#[test]
fn polygon_prism_decomposition_rejects_non_simple_polygon() {
    let prism = PolygonPrism {
        height: 1.0,
        vertices: vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(2.0, 0.0),
        ],
    };

    assert!(matches!(
        prism.decompose(),
        Err(PolygonPrismDecompositionError::NotSimple { .. })
    ));
}

#[test]
fn shape_local_bounds_query_resolves_shape_components() {
    let mut world = World::new();
    let entity = world
        .spawn(BoxShapeComponent {
            configuration: BoxShapeConfig {
                dimensions: Vec3::new(4.0, 2.0, 6.0),
            },
            ..Default::default()
        })
        .id();

    let mut system_state = SystemState::<ShapeLocalBoundsQuery>::new(&mut world);
    let shapes = system_state.get(&world).unwrap();
    let bounds = shapes.local_bounds(entity).unwrap();

    assert_eq!(bounds.min, Vec3::new(-2.0, -1.0, -3.0).into());
    assert_eq!(bounds.max, Vec3::new(2.0, 1.0, 3.0).into());
}
