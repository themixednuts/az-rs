use super::*;
use crate::{
    DescriptorWeightSelectorConfig, DistributionFilterConfig, FilterStage, InstanceData,
    InstanceSpawner, LegacyVegetationInstanceSpawner, PositionModifierConfig, ScaleModifierConfig,
    SortBehavior, SurfaceAltitudeFilterConfig, VegetationDescriptor,
    VegetationDescriptorListComponent, VegetationDescriptorListConfig, VegetationPlugin,
    VegetationSurfaceTag, VegetationSurfaceTagWeight,
};
use az_gem_gradient_signal::{
    ConstantGradientComponent, ConstantGradientConfig, GradientLookup, GradientSampleParams,
    GradientSampler, GradientSourceQuery,
};
use az_gem_legacy_terrain::{
    RegionHeightmap, TerrainRegionAsset, TerrainRegionId, TerrainSurfaceWeight, TerrainWorld,
};
use bevy::ecs::system::SystemState;
use bevy::math::Vec3A;
use bevy::math::bounding::Aabb3d;

#[test]
// `float_cmp`: 20/16 is exactly 1.25 in binary, and the assertion is that the default config
// yields that exact ratio.
#[allow(clippy::float_cmp)]
fn area_system_defaults_match_lumberyard_source() {
    let config = AreaSystemConfig::default();

    assert_eq!(config.view_rectangle_size, 13);
    assert_eq!(config.sector_density, 20);
    assert_eq!(config.sector_size_in_meters, 16);
    assert_eq!(config.thread_processing_interval_ms, 500);
    assert_eq!(config.sector_point_snap_mode, SnapMode::Corner);
    assert_eq!(config.instances_per_sector(), 400);
    assert_eq!(config.points_per_meter(), 1.25);
}

#[test]
fn vegetation_area_component_defaults_match_public_registration() {
    assert_eq!(
        VEGETATION_AREA_COMPONENT_TYPE_ID,
        uuid::uuid!("ADCA8AB2-924D-463F-A0EA-27D254F9682A")
    );
    assert_eq!(
        VEGETATION_AREA_CONFIG_TYPE_ID,
        uuid::uuid!("1272ADB3-75A0-46FB-94CE-2C82D3741F09")
    );

    let config = VegetationAreaConfig::default();

    assert_eq!(config.area_type.raw(), 0);
    assert_eq!(config.area_type.layer(), FOREGROUND_LAYER);
    assert_eq!(config.placement_mode_type_flag.bits(), 0);
    assert_eq!(config.priority, 1);
    assert_eq!(config.area_config().priority, PRIORITY_MIN);

    let coverage = VegetationAreaConfig {
        area_type: VegetationAreaType::COVERAGE,
        priority: 8,
        ..Default::default()
    };
    assert_eq!(coverage.area_config().layer, BACKGROUND_LAYER);
    assert_eq!(coverage.area_config().priority, 7);
}

#[test]
fn vegetation_blocker_defaults_match_lumberyard_source() {
    assert_eq!(
        VEGETATION_BLOCKER_COMPONENT_TYPE_ID,
        uuid::uuid!("954683F7-7965-4686-BCDB-0F65767730D3")
    );
    assert_eq!(
        VEGETATION_BLOCKER_CONFIG_TYPE_ID,
        uuid::uuid!("DA4B8865-B9F6-48DF-A789-7057B18BB883")
    );

    let config = VegetationBlockerConfig::default();

    assert_eq!(config.area.layer, FOREGROUND_LAYER);
    assert_eq!(config.area.priority, PRIORITY_MAX);
    assert!(config.inherit_behavior);
    assert!(!config.use_relative_uvw);
}

#[test]
fn area_blender_defaults_match_current_registration() {
    assert_eq!(
        VEGETATION_AREA_BLENDER_COMPONENT_TYPE_ID,
        uuid::uuid!("7C051A94-079C-45A5-A94D-2005D2440A1E")
    );
    assert_eq!(
        VEGETATION_AREA_BLENDER_CONFIG_TYPE_ID,
        uuid::uuid!("1AC67B48-C6B1-412B-9290-662A01CD384B")
    );

    let component = AreaBlenderComponent::default();
    let config = component.configuration;

    assert_eq!(config.area, AreaConfig::default());
    assert!(config.inherit_behavior);
    assert!(config.propagate_behavior);
    assert!(config.operations.is_empty());
}

#[test]
fn reference_shape_defaults_match_current_registration() {
    assert_eq!(
        VEGETATION_REFERENCE_SHAPE_COMPONENT_TYPE_ID,
        uuid::uuid!("AE52A11C-3356-4178-89A2-7152E9FC869E")
    );
    assert_eq!(
        VEGETATION_REFERENCE_SHAPE_CONFIG_TYPE_ID,
        uuid::uuid!("BDBD474B-23E0-4722-AAF1-8CCCC98D8C5C")
    );

    let component = ReferenceShapeComponent::default();

    assert_eq!(component.configuration.shape_entity, None);
}

#[test]
fn sector_point_grid_uses_corner_snap_mode() {
    let config = AreaSystemConfig {
        sector_density: 2,
        sector_size_in_meters: 4,
        sector_point_snap_mode: SnapMode::Corner,
        ..Default::default()
    };
    let sector_bounds =
        Aabb3d::from_min_max(Vec3A::new(10.0, 1.0, 20.0), Vec3A::new(14.0, 1.0, 24.0));

    let points = config
        .sector_point_grid(sector_bounds)
        .unwrap()
        .collect::<Vec<_>>();

    assert_eq!(
        points,
        vec![
            Vec3::new(10.0, 1.0, 20.0),
            Vec3::new(12.0, 1.0, 20.0),
            Vec3::new(10.0, 1.0, 22.0),
            Vec3::new(12.0, 1.0, 22.0),
        ]
    );
}

#[test]
fn sector_point_grid_uses_center_snap_mode() {
    let config = AreaSystemConfig {
        sector_density: 2,
        sector_size_in_meters: 4,
        sector_point_snap_mode: SnapMode::Center,
        ..Default::default()
    };
    let sector_bounds =
        Aabb3d::from_min_max(Vec3A::new(10.0, 1.0, 20.0), Vec3A::new(14.0, 1.0, 24.0));

    let points = config
        .sector_point_grid(sector_bounds)
        .unwrap()
        .collect::<Vec<_>>();

    assert_eq!(
        points,
        vec![
            Vec3::new(11.0, 1.0, 21.0),
            Vec3::new(13.0, 1.0, 21.0),
            Vec3::new(11.0, 1.0, 23.0),
            Vec3::new(13.0, 1.0, 23.0),
        ]
    );
}

#[test]
fn sector_point_grid_rejects_invalid_density() {
    let config = AreaSystemConfig {
        sector_density: 0,
        ..Default::default()
    };
    let sector_bounds = Aabb3d::from_min_max(Vec3A::ZERO, Vec3A::splat(1.0));

    assert!(config.sector_point_grid(sector_bounds).is_none());
}

#[test]
fn sector_id_at_world_uses_bevy_xz_axes() {
    let config = AreaSystemConfig {
        sector_size_in_meters: 16,
        ..Default::default()
    };

    assert_eq!(
        config.sector_id_at_world(Vec3::new(31.9, 50.0, -0.1)),
        Some(SectorId::new(1, -1))
    );
    assert_eq!(
        config.sector_id_at_world(Vec3::new(32.0, 50.0, 16.0)),
        Some(SectorId::new(2, 1))
    );
}

#[test]
fn sector_bounds_use_bevy_xz_axes() {
    let config = AreaSystemConfig {
        sector_size_in_meters: 16,
        ..Default::default()
    };

    assert_eq!(
        config.sector_bounds(SectorId::new(2, -1)),
        Some(Aabb3d::from_min_max(
            Vec3A::new(32.0, 0.0, -16.0),
            Vec3A::new(48.0, 0.0, 0.0)
        ))
    );
}

#[test]
fn view_rect_tracks_camera_sector_window() {
    let config = AreaSystemConfig {
        view_rectangle_size: 5,
        sector_size_in_meters: 10,
        ..Default::default()
    };

    let view = config.view_rect_at(Vec3::new(40.0, 7.0, 50.0)).unwrap();

    assert_eq!(view.min_sector(), SectorId::new(2, 3));
    assert_eq!(view.max_sector(), SectorId::new(6, 7));
    assert_eq!(view.num_sectors(), 25);
    assert!(view.is_inside(SectorId::new(4, 5)));
    assert!(!view.is_inside(SectorId::new(7, 5)));
    assert_eq!(
        view.sector_ids().take(3).collect::<Vec<_>>(),
        vec![
            SectorId::new(2, 3),
            SectorId::new(3, 3),
            SectorId::new(4, 3)
        ]
    );
    assert_eq!(
        view.bounds,
        Aabb3d::from_min_max(Vec3A::new(20.0, 0.0, 30.0), Vec3A::new(70.0, 0.0, 80.0))
    );
}

#[test]
fn view_rect_overlap_clips_sector_ranges() {
    let a = ViewRect::new(2, 3, 5, 5, 10);
    let b = ViewRect::new(4, 1, 3, 4, 10);

    let overlap = a.overlap(&b, 10);

    assert_eq!(overlap.min_sector(), SectorId::new(4, 3));
    assert_eq!(overlap.width, 3);
    assert_eq!(overlap.height, 2);
    assert_eq!(overlap.num_sectors(), 6);
}

#[test]
fn claim_handles_are_stable_for_sector_points() {
    let handle = ClaimHandle::for_sector_index(SectorId::new(3, 4), 7);

    assert_eq!(
        handle,
        ClaimHandle::for_sector_index(SectorId::new(3, 4), 7)
    );
    assert_ne!(
        handle,
        ClaimHandle::for_sector_index(SectorId::new(3, 4), 8)
    );
    assert_ne!(
        handle,
        ClaimHandle::for_sector_index(SectorId::new(4, 3), 7)
    );
}

#[test]
fn terrain_sector_claim_context_uses_loaded_surface_points() {
    let mut app = App::new();
    app.init_resource::<Assets<TerrainRegionAsset>>();

    let heightmap = RegionHeightmap::from_samples(vec![5, 5, 5, 5], 2, 1.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        region: TerrainRegionId::new(0, 0),
        origin: Vec2::new(100.0, 200.0),
        heightmap,
        surface_resolution: 2,
        surface_weights: vec![
            TerrainSurfaceWeight::HOLE,
            TerrainSurfaceWeight::default(),
            TerrainSurfaceWeight::default(),
            TerrainSurfaceWeight::default(),
        ],
        ..Default::default()
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<TerrainRegionAsset>>()
        .add(region);
    let mut terrain_world = TerrainWorld::default();
    terrain_world
        .loaded_regions
        .insert(TerrainRegionId::new(0, 0), handle);

    let config = AreaSystemConfig {
        sector_density: 2,
        sector_size_in_meters: 2,
        ..Default::default()
    };
    let terrain_assets = app.world().resource::<Assets<TerrainRegionAsset>>();

    let context = ClaimContext::from_terrain_sector(
        &config,
        SectorId::new(50, 100),
        &terrain_world,
        terrain_assets,
    )
    .unwrap();

    assert_eq!(context.available_points.len(), 4);
    assert_eq!(
        context.available_points[0].position,
        Vec3::new(100.0, 5.0, 200.0)
    );
    assert_eq!(context.available_points[0].normal, Vec3::Y);
    assert_eq!(
        context.available_points[0].masks,
        vec![VegetationSurfaceTagWeight::new(
            VegetationSurfaceTag::TERRAIN_HOLE,
            1.0
        )]
    );
    assert_eq!(
        context.available_points[1].masks,
        vec![VegetationSurfaceTagWeight::new(
            VegetationSurfaceTag::TERRAIN,
            1.0
        )]
    );
    assert!(context.masks.contains(&VegetationSurfaceTagWeight::new(
        VegetationSurfaceTag::TERRAIN_HOLE,
        1.0
    )));
    assert!(context.masks.contains(&VegetationSurfaceTagWeight::new(
        VegetationSurfaceTag::TERRAIN,
        1.0
    )));

    let instance = context.available_points[0].instance_data();
    assert_eq!(instance.position, context.available_points[0].position);
    assert_eq!(instance.normal, context.available_points[0].normal);
    assert_eq!(instance.masks, context.available_points[0].masks);
}

#[test]
fn spawner_claims_points_with_selectable_descriptors() {
    let mut context = ClaimContext {
        masks: vec![VegetationSurfaceTagWeight::new(
            VegetationSurfaceTag::TERRAIN,
            1.0,
        )],
        available_points: vec![
            ClaimPoint {
                handle: ClaimHandle(1),
                position: Vec3::new(1.0, 2.0, 3.0),
                normal: Vec3::Y,
                masks: vec![VegetationSurfaceTagWeight::new(
                    VegetationSurfaceTag::TERRAIN,
                    1.0,
                )],
            },
            ClaimPoint {
                handle: ClaimHandle(2),
                position: Vec3::new(4.0, 5.0, 6.0),
                normal: Vec3::Y,
                masks: vec![VegetationSurfaceTagWeight::new(
                    VegetationSurfaceTag::TERRAIN,
                    1.0,
                )],
            },
        ],
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![
            VegetationDescriptor::default(),
            VegetationDescriptor {
                instance_spawner: InstanceSpawner::LegacyVegetation(
                    LegacyVegetationInstanceSpawner {
                        mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                        ..Default::default()
                    },
                ),
                ..Default::default()
            },
        ],
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };

    let instances = spawner.claim_positions(None, &mut context, &descriptors);

    assert_eq!(instances.len(), 2);
    assert!(context.available_points.is_empty());
    assert_eq!(instances[0].position, Vec3::new(1.0, 2.0, 3.0));
    assert_eq!(instances[0].normal, Vec3::Y);
    assert_eq!(
        instances[0].masks,
        vec![VegetationSurfaceTagWeight::new(
            VegetationSurfaceTag::TERRAIN,
            1.0
        )]
    );
    assert_eq!(instances[0].descriptor_index, Some(1));
}

#[test]
fn spawner_selects_descriptors_through_weight_selector() {
    let mut context = ClaimContext {
        available_points: vec![ClaimPoint {
            handle: ClaimHandle(1),
            position: Vec3::ZERO,
            normal: Vec3::Y,
            masks: Vec::new(),
        }],
        ..Default::default()
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![
            VegetationDescriptor {
                instance_spawner: InstanceSpawner::LegacyVegetation(
                    LegacyVegetationInstanceSpawner {
                        mesh_asset_path: Some("Objects/Nature/Low.cgf".to_string()),
                        ..Default::default()
                    },
                ),
                weight: 1.0,
                ..Default::default()
            },
            VegetationDescriptor {
                instance_spawner: InstanceSpawner::LegacyVegetation(
                    LegacyVegetationInstanceSpawner {
                        mesh_asset_path: Some("Objects/Nature/High.cgf".to_string()),
                        ..Default::default()
                    },
                ),
                weight: 5.0,
                ..Default::default()
            },
        ],
    };
    let selector = DescriptorWeightSelectorConfig {
        sort_behavior: SortBehavior::Descending,
        ..Default::default()
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };

    let instances = spawner.claim_positions_with_gradient_sources(
        None,
        &mut context,
        &descriptors,
        SpawnerProcessingSet {
            selector: Some(&selector),
            ..Default::default()
        },
        &FixedGradient(0.0),
    );

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].descriptor_index, Some(1));
}

#[test]
fn spawner_leaves_unclaimed_points_when_descriptors_are_not_selectable() {
    let mut context = ClaimContext {
        available_points: vec![ClaimPoint {
            handle: ClaimHandle(1),
            position: Vec3::ZERO,
            normal: Vec3::Y,
            masks: Vec::new(),
        }],
        ..Default::default()
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![VegetationDescriptor::default()],
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };

    let instances = spawner.claim_positions(None, &mut context, &descriptors);

    assert!(instances.is_empty());
    assert_eq!(context.available_points.len(), 1);
}

#[test]
fn spawner_filters_claims_before_removing_points() {
    let mut context = ClaimContext {
        available_points: vec![ClaimPoint {
            handle: ClaimHandle(1),
            position: Vec3::new(0.0, 3.0, 0.0),
            normal: Vec3::Y,
            masks: Vec::new(),
        }],
        ..Default::default()
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![VegetationDescriptor {
            instance_spawner: InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
                mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }],
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };
    let altitude = SurfaceAltitudeFilterConfig {
        altitude_min: 10.0,
        altitude_max: 12.0,
        ..Default::default()
    };

    let instances = spawner.claim_positions_with_filters(
        None,
        &mut context,
        &descriptors,
        SpawnerFilterSet {
            altitude: Some(&altitude),
            ..Default::default()
        },
    );

    assert!(instances.is_empty());
    assert_eq!(context.available_points.len(), 1);
}

#[test]
// `float_cmp`: The scale modifier's exact output is the property under test.
#[allow(clippy::float_cmp)]
fn spawner_applies_modifiers_between_pre_and_post_filters() {
    let mut context = ClaimContext {
        available_points: vec![ClaimPoint {
            handle: ClaimHandle(1),
            position: Vec3::new(0.0, 3.0, 0.0),
            normal: Vec3::Y,
            masks: Vec::new(),
        }],
        ..Default::default()
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![VegetationDescriptor {
            instance_spawner: InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
                mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }],
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };
    let position = PositionModifierConfig {
        range_min: Vec3::new(2.0, 10.0, 4.0),
        range_max: Vec3::new(2.0, 10.0, 4.0),
        ..Default::default()
    };
    let scale = ScaleModifierConfig {
        range_min: 2.0,
        range_max: 2.0,
        ..Default::default()
    };
    let altitude = SurfaceAltitudeFilterConfig {
        filter_stage: FilterStage::PostProcess,
        altitude_min: 13.0,
        altitude_max: 13.0,
        ..Default::default()
    };

    let instances = spawner.claim_positions_with_processing(
        None,
        &mut context,
        &descriptors,
        SpawnerProcessingSet {
            filters: SpawnerFilterSet {
                altitude: Some(&altitude),
                ..Default::default()
            },
            modifiers: SpawnerModifierSet {
                position: Some(&position),
                scale: Some(&scale),
                ..Default::default()
            },
            ..Default::default()
        },
    );

    assert_eq!(instances.len(), 1);
    assert!(context.available_points.is_empty());
    assert_eq!(instances[0].position, Vec3::new(2.0, 13.0, 4.0));
    assert_eq!(instances[0].scale, 2.0);
}

#[test]
// `float_cmp`: The gradient-driven scale must come out exactly 2.0, not near it.
#[allow(clippy::float_cmp)]
fn spawner_modifiers_sample_gradient_sources() {
    let mut world = World::new();
    let gradient = world
        .spawn(ConstantGradientComponent {
            configuration: ConstantGradientConfig { value: 0.5 },
        })
        .id();
    let scale = ScaleModifierConfig {
        range_min: 1.0,
        range_max: 3.0,
        gradient: GradientSampler {
            gradient: Some(gradient),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state.get(&world).unwrap();
    let mut context = ClaimContext {
        available_points: vec![ClaimPoint {
            handle: ClaimHandle(1),
            position: Vec3::ZERO,
            normal: Vec3::Y,
            masks: Vec::new(),
        }],
        ..Default::default()
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![VegetationDescriptor {
            instance_spawner: InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
                mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }],
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };

    let instances = spawner.claim_positions_with_gradient_sources(
        None,
        &mut context,
        &descriptors,
        SpawnerProcessingSet {
            modifiers: SpawnerModifierSet {
                scale: Some(&scale),
                ..Default::default()
            },
            ..Default::default()
        },
        &gradients,
    );

    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].scale, 2.0);
}

#[test]
fn spawner_distribution_filter_samples_gradient_sources() {
    let mut world = World::new();
    let gradient = world
        .spawn(ConstantGradientComponent {
            configuration: ConstantGradientConfig { value: 0.5 },
        })
        .id();
    let distribution = DistributionFilterConfig {
        filter_stage: FilterStage::PreProcess,
        threshold_min: 0.6,
        threshold_max: 1.0,
        gradient: GradientSampler {
            gradient: Some(gradient),
            ..Default::default()
        },
    };
    let mut system_state = SystemState::<GradientSourceQuery>::new(&mut world);
    let gradients = system_state.get(&world).unwrap();
    let mut context = ClaimContext {
        available_points: vec![ClaimPoint {
            handle: ClaimHandle(1),
            position: Vec3::ZERO,
            normal: Vec3::Y,
            masks: Vec::new(),
        }],
        ..Default::default()
    };
    let descriptors = VegetationDescriptorListConfig {
        vegetation_descriptors: vec![VegetationDescriptor {
            instance_spawner: InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
                mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        }],
    };
    let spawner = SpawnerComponent {
        configuration: SpawnerConfig {
            allow_empty_meshes: false,
            ..Default::default()
        },
    };

    let instances = spawner.claim_positions_with_gradient_sources(
        None,
        &mut context,
        &descriptors,
        SpawnerProcessingSet {
            filters: SpawnerFilterSet {
                distribution: Some(&distribution),
                ..Default::default()
            },
            ..Default::default()
        },
        &gradients,
    );

    assert!(instances.is_empty());
    assert_eq!(context.available_points.len(), 1);
}

#[test]
// `float_cmp`: The fixture writes the terrain height as exactly 3.0 and it reaches the
// instance unmodified.
#[allow(clippy::float_cmp)]
fn plugin_spawns_terrain_sector_instances_from_area_spawner() {
    let mut app = App::new();
    app.insert_resource(AreaSystemConfig {
        view_rectangle_size: 1,
        sector_density: 2,
        sector_size_in_meters: 2,
        ..Default::default()
    });
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<TerrainRegionAsset>>()
        .add_plugins(VegetationPlugin);

    let heightmap = RegionHeightmap::from_samples(vec![3, 3, 3, 3], 2, 1.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        region: TerrainRegionId::new(0, 0),
        origin: Vec2::ZERO,
        heightmap,
        surface_resolution: 1,
        surface_weights: vec![TerrainSurfaceWeight::default()],
        ..Default::default()
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<TerrainRegionAsset>>()
        .add(region);
    let mut terrain_world = TerrainWorld::default();
    terrain_world
        .loaded_regions
        .insert(TerrainRegionId::new(0, 0), handle);
    app.insert_resource(terrain_world);

    let area = app
        .world_mut()
        .spawn((
            SpawnerComponent {
                configuration: SpawnerConfig {
                    allow_empty_meshes: false,
                    ..Default::default()
                },
            },
            VegetationDescriptorListComponent {
                configuration: VegetationDescriptorListConfig {
                    vegetation_descriptors: vec![VegetationDescriptor {
                        instance_spawner: InstanceSpawner::LegacyVegetation(
                            LegacyVegetationInstanceSpawner {
                                mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
                                ..Default::default()
                            },
                        ),
                        ..Default::default()
                    }],
                },
            },
        ))
        .id();
    app.world_mut()
        .spawn((Camera::default(), Transform::from_xyz(1.0, 10.0, 1.0)));

    app.update();
    app.update();

    let instances = {
        let world = app.world_mut();
        let mut query = world.query::<(&InstanceData, &VegetationSectorInstance)>();
        query
            .iter(world)
            .map(|(instance, sector_instance)| (instance.clone(), *sector_instance))
            .collect::<Vec<_>>()
    };

    assert_eq!(instances.len(), 4);
    assert!(
        instances
            .iter()
            .all(|(_, sector_instance)| sector_instance.area == area)
    );
    assert!(
        instances
            .iter()
            .all(|(_, sector_instance)| sector_instance.sector == SectorId::ZERO)
    );
    assert!(
        instances
            .iter()
            .all(|(instance, _)| instance.entity == Some(area))
    );
    assert!(
        instances
            .iter()
            .all(|(instance, _)| instance.descriptor_index == Some(0))
    );
    assert!(
        instances
            .iter()
            .all(|(instance, _)| instance.position.y == 3.0)
    );

    app.update();
    let count_after_second_update = {
        let world = app.world_mut();
        let mut query = world.query_filtered::<Entity, With<VegetationSectorInstance>>();
        query.iter(world).count()
    };
    assert_eq!(count_after_second_update, 4);
}

#[test]
fn plugin_fills_sector_by_area_priority_with_shared_claims() {
    let mut app = App::new();
    app.insert_resource(AreaSystemConfig {
        view_rectangle_size: 1,
        sector_density: 2,
        sector_size_in_meters: 2,
        ..Default::default()
    });
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<TerrainRegionAsset>>()
        .add_plugins(VegetationPlugin);

    let heightmap = RegionHeightmap::from_samples(vec![3, 3, 3, 3], 2, 1.0, 1.0, 0.0).unwrap();
    let region = TerrainRegionAsset {
        region: TerrainRegionId::new(0, 0),
        origin: Vec2::ZERO,
        heightmap,
        surface_resolution: 1,
        surface_weights: vec![TerrainSurfaceWeight::default()],
        ..Default::default()
    };
    let handle = app
        .world_mut()
        .resource_mut::<Assets<TerrainRegionAsset>>()
        .add(region);
    let mut terrain_world = TerrainWorld::default();
    terrain_world
        .loaded_regions
        .insert(TerrainRegionId::new(0, 0), handle);
    app.insert_resource(terrain_world);

    let descriptor = VegetationDescriptor {
        instance_spawner: InstanceSpawner::LegacyVegetation(LegacyVegetationInstanceSpawner {
            mesh_asset_path: Some("Objects/Nature/Oak.cgf".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let low_area = spawn_test_area(&mut app, 1, descriptor.clone());
    let high_area = spawn_test_area(&mut app, 8, descriptor);
    app.world_mut()
        .spawn((Camera::default(), Transform::from_xyz(1.0, 10.0, 1.0)));

    app.update();
    app.update();

    let instances = {
        let world = app.world_mut();
        let mut query = world.query::<(&InstanceData, &VegetationSectorInstance)>();
        query
            .iter(world)
            .map(|(instance, sector_instance)| (instance.clone(), *sector_instance))
            .collect::<Vec<_>>()
    };

    assert_eq!(instances.len(), 4);
    assert!(
        instances
            .iter()
            .all(|(_, sector_instance)| sector_instance.area == high_area)
    );
    assert!(
        instances
            .iter()
            .all(|(_, sector_instance)| sector_instance.area != low_area)
    );
}

fn spawn_test_area(app: &mut App, priority: i32, descriptor: VegetationDescriptor) -> Entity {
    app.world_mut()
        .spawn((
            VegetationAreaComponent {
                configuration: VegetationAreaConfig {
                    priority,
                    ..Default::default()
                },
            },
            SpawnerComponent {
                configuration: SpawnerConfig {
                    allow_empty_meshes: false,
                    ..Default::default()
                },
            },
            VegetationDescriptorListComponent {
                configuration: VegetationDescriptorListConfig {
                    vegetation_descriptors: vec![descriptor],
                },
            },
        ))
        .id()
}

#[test]
fn spawner_and_descriptor_list_defaults_match_public_registration() {
    let spawner = SpawnerComponent::default();
    let descriptor_list = VegetationDescriptorListComponent::default();

    assert_eq!(spawner.configuration.area.layer, FOREGROUND_LAYER);
    assert_eq!(spawner.configuration.area.priority, PRIORITY_MIN);
    assert!(spawner.configuration.inherit_behavior);
    assert!(!spawner.configuration.use_relative_uvw);
    assert!(spawner.configuration.allow_empty_meshes);
    assert_eq!(spawner.configuration.filter_stage, FilterStage::PreProcess);

    assert!(descriptor_list.configuration.is_empty());
    assert_eq!(descriptor_list.configuration.descriptor_count(), 0);
}

struct FixedGradient(f32);

impl GradientLookup for FixedGradient {
    fn sample_gradient(&self, _sampler: &GradientSampler, _params: GradientSampleParams) -> f32 {
        self.0
    }
}
