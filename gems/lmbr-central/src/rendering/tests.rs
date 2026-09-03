use super::*;
use crate::{LmbrCentralAssetPlugin, LmbrCentralPlugin, MaterialAssetBinding};
use bevy::asset::AssetPlugin;
use bevy::light::{FogVolume as BevyFogVolume, LightProbe as BevyLightProbe};
use bevy::mesh::Mesh3d;

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn fog_volume_configuration_defaults_match_lumberyard_source() {
    let config = FogVolumeConfiguration::default();

    assert_eq!(config.min_spec, EngineSpec::Low);
    assert_eq!(config.view_distance_multiplier, 1.0);
    assert_eq!(config.volume_type, FogVolumeType::Ellipsoid);
    assert_eq!(config.color, Color::WHITE);
    assert_eq!(config.size, Vec3::ONE);
    assert_eq!(config.hdr_dynamic, 0.0);
    assert!(!config.use_global_fog_color);
    assert_eq!(config.global_density, 1.0);
    assert_eq!(config.density_offset, 0.0);
    assert_eq!(config.near_cutoff, 0.0);
    assert_eq!(config.fall_off_dir_long, 0.0);
    assert_eq!(config.fall_off_dir_latitude, 90.0);
    assert_eq!(config.fall_off_shift, 0.0);
    assert_eq!(config.fall_off_scale, 1.0);
    assert_eq!(config.soft_edges, 1.0);
    assert_eq!(config.ramp_start, 1.0);
    assert_eq!(config.ramp_end, 50.0);
    assert_eq!(config.ramp_influence, 0.0);
    assert_eq!(config.wind_influence, 1.0);
    assert_eq!(config.density_noise_scale, 1.0);
    assert_eq!(config.density_noise_offset, 1.0);
    assert_eq!(config.density_noise_time_frequency, 0.0);
    assert_eq!(config.density_noise_frequency, Vec3::splat(10.0));
    assert!(!config.ignores_vis_areas);
    assert!(!config.affects_this_area_only);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn fog_volume_configuration_maps_to_bevy_fog_volume() {
    let config = FogVolumeConfiguration {
        color: Color::srgba(0.25, 0.5, 0.75, 1.0),
        global_density: 0.0,
        hdr_dynamic: 2.5,
        ..Default::default()
    };

    let fog_volume = config.bevy_fog_volume();

    assert_eq!(fog_volume.fog_color, config.color);
    assert_eq!(fog_volume.light_tint, config.color);
    assert_eq!(fog_volume.density_factor, 0.01);
    assert_eq!(fog_volume.light_intensity, 2.5);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn lens_flare_configuration_defaults_match_lumberyard_source() {
    let config = LensFlareConfiguration::default();

    assert!(config.visible);
    assert!(config.on_initially);
    assert_eq!(config.min_spec, EngineSpec::Low);
    assert_eq!(config.frustum_angle_degrees, 360.0);
    assert_eq!(config.size, 1.0);
    assert!(!config.attach_to_sun);
    assert!(config.affects_this_area_only);
    assert!(!config.ignore_vis_areas);
    assert!(!config.indoor_only);
    assert_eq!(config.view_distance_multiplier, 1.0);
    assert_eq!(config.tint, Color::WHITE);
    assert_eq!(config.brightness, 1.0);
    assert!(!config.sync_anim_with_light);
    assert_eq!(config.anim_index, 0);
    assert_eq!(config.anim_speed, 1.0);
    assert_eq!(config.anim_phase, 0.0);
}

#[test]
fn lens_flare_preview_material_is_emissive() {
    let config = LensFlareConfiguration {
        tint: Color::srgba(0.2, 0.4, 0.8, 0.5),
        brightness: 3.0,
        ..Default::default()
    };

    let material = config.preview_material();
    let tint = config.tint.to_linear();

    assert_eq!(material.base_color, config.tint);
    assert_eq!(material.emissive, tint * 3.0);
    assert!(material.unlit);
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn particle_emitter_settings_defaults_match_lumberyard_source() {
    let settings = ParticleEmitterSettings::default();

    assert!(settings.visible);
    assert!(settings.enable);
    assert!(!settings.pre_roll);
    assert_eq!(settings.color, LinearRgba::WHITE);
    assert_eq!(settings.particle_count_scale, 1.0);
    assert_eq!(settings.time_scale, 1.0);
    assert_eq!(settings.speed_scale, 1.0);
    assert_eq!(settings.global_size_scale, 1.0);
    assert_eq!(settings.pulse_period, 0.0);
    assert_eq!(settings.particle_size(), Vec3::ONE);
    assert_eq!(settings.particle_size_random, 0.0);
    assert_eq!(settings.strength, -1.0);
    assert!(!settings.ignore_rotation);
    assert!(!settings.not_attached);
    assert!(!settings.register_by_bounding_box);
    assert!(settings.use_lod);
    assert!(!settings.enable_audio);
    assert_eq!(settings.view_distance_multiplier, 1.0);
    assert!(settings.use_vis_area);
    assert!(!settings.is_rendered());
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn particle_emitter_preview_uses_tint_and_size_scale() {
    let settings = ParticleEmitterSettings {
        selected_emitter: "fx/fire/sparks".to_string(),
        color: LinearRgba::new(1.0, 0.25, 0.1, 0.5),
        alpha_scale: 0.5,
        global_size_scale: 2.0,
        particle_size_x: 1.0,
        particle_size_y: 3.0,
        particle_size_z: 2.0,
        ..Default::default()
    };

    let material = settings.preview_material();

    assert!(settings.is_rendered());
    assert_eq!(settings.preview_size(), 6.0);
    assert_eq!(material.base_color.to_srgba().alpha, 0.25);
    assert!(material.unlit);
    assert_eq!(material.alpha_mode, AlphaMode::Blend);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn high_quality_shadow_config_defaults_match_lumberyard_source() {
    let config = HighQualityShadowConfig::default();

    assert!(config.enabled);
    assert!(config.is_enabled());
    assert_eq!(config.const_bias, 0.001);
    assert_eq!(config.slope_bias, 0.01);
    assert_eq!(config.jitter, 0.01);
    assert_eq!(config.bbox_scale, Vec3::ONE);
    assert_eq!(config.shadow_map_size, 1024);
    assert_eq!(config.shadow_map_size(), 1024);
}

#[test]
fn high_quality_shadow_config_clamps_runtime_shadow_map_size() {
    let config = HighQualityShadowConfig {
        shadow_map_size: 0,
        ..Default::default()
    };

    assert_eq!(config.shadow_map_size(), 1);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn decal_configuration_defaults_match_lumberyard_source() {
    let config = DecalConfiguration::default();

    assert_eq!(config.projection_type, DecalProjectionType::Planar);
    assert!(config.visible);
    assert_eq!(config.sort_priority, 16);
    assert_eq!(config.depth, 1.0);
    assert_eq!(config.offset, Vec3::ZERO);
    assert_eq!(config.opacity, 1.0);
    assert!(!config.deferred);
    assert_eq!(config.max_view_distance, 8000.0);
    assert_eq!(config.view_distance_multiplier, 1.0);
    assert_eq!(config.min_spec, EngineSpec::Low);
}

#[test]
fn decal_projection_type_maps_native_values() {
    assert_eq!(
        DecalProjectionType::from_native_value(0),
        Some(DecalProjectionType::Planar)
    );
    assert_eq!(
        DecalProjectionType::ProjectOnTerrainAndStaticObjects.native_value(),
        2
    );
    assert_eq!(DecalProjectionType::from_native_value(3), None);
}

#[test]
fn engine_spec_maps_native_values() {
    assert_eq!(EngineSpec::from_native_value(0), Some(EngineSpec::Low));
    assert_eq!(EngineSpec::from_native_value(1), Some(EngineSpec::Low));
    assert_eq!(EngineSpec::from_native_value(4), Some(EngineSpec::VeryHigh));
    assert_eq!(
        EngineSpec::from_native_value(u32::MAX),
        Some(EngineSpec::Never)
    );
    assert_eq!(EngineSpec::High.native_value(), 3);
    assert_eq!(EngineSpec::from_native_value(5), None);
}

#[test]
fn light_type_maps_native_values() {
    assert_eq!(LightType::from_native_value(0), Some(LightType::Point));
    assert_eq!(LightType::from_native_value(2), Some(LightType::Projector));
    assert_eq!(LightType::Probe.native_value(), 3);
    assert_eq!(LightType::from_native_value(4), None);
}

#[test]
fn light_cubemap_resolution_maps_native_values() {
    assert_eq!(
        LightCubemapResolution::from_native_value(128),
        Some(LightCubemapResolution::Res128)
    );
    assert_eq!(LightCubemapResolution::Res512.native_value(), 512);
    assert_eq!(LightCubemapResolution::from_native_value(1024), None);
}

#[test]
fn voxel_gi_mode_maps_native_values() {
    assert_eq!(VoxelGiMode::from_native_value(0), Some(VoxelGiMode::None));
    assert_eq!(
        VoxelGiMode::from_native_value(2),
        Some(VoxelGiMode::Dynamic)
    );
    assert_eq!(VoxelGiMode::Static.native_value(), 1);
    assert_eq!(VoxelGiMode::from_native_value(3), None);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn light_configuration_defaults_match_lumberyard_source() {
    let config = LightConfiguration::default();

    assert_eq!(config.light_type, LightType::Point);
    assert!(config.visible);
    assert!(config.on_initially);
    assert_eq!(config.point_max_distance, 2.0);
    assert_eq!(config.point_attenuation_bulb_size, 0.05);
    assert_eq!(config.area_width, 5.0);
    assert_eq!(config.area_height, 5.0);
    assert_eq!(config.projector_range, 5.0);
    assert_eq!(config.projector_fov_degrees, 90.0);
    assert_eq!(config.probe_area, Vec3::splat(20.0));
    assert_eq!(config.probe_cubemap_resolution.pixels(), 256);
    assert_eq!(config.cast_shadows_spec, EngineSpec::Never);
    assert_eq!(config.min_spec, EngineSpec::Low);
    assert!(config.affects_this_area_only);
    assert!(config.volumetric_fog);
    assert!(!config.cast_terrain_shadows);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn light_probe_configuration_maps_to_bevy_probe_data() {
    let config = LightConfiguration {
        light_type: LightType::Probe,
        probe_area: Vec3::new(8.0, 4.0, 2.0),
        diffuse_multiplier: 0.5,
        specular_multiplier: 1.5,
        ..Default::default()
    };

    let environment_map =
        config.environment_map_light(Handle::<Image>::default(), Handle::<Image>::default());

    assert_eq!(config.probe_transform_scale(), Vec3::new(8.0, 4.0, 2.0));
    assert_eq!(config.environment_map_intensity(), 1.5);
    assert_eq!(environment_map.intensity, 1.5);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn plugin_syncs_fog_volume_component_to_bevy_fog_volume() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(FogVolumeComponent {
            configuration: FogVolumeConfiguration {
                color: Color::srgba(0.1, 0.2, 0.3, 1.0),
                size: Vec3::new(2.0, 3.0, 4.0),
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    let fog_volume = entity_ref.get::<BevyFogVolume>().unwrap();
    assert_eq!(fog_volume.fog_color, Color::srgba(0.1, 0.2, 0.3, 1.0));
    assert_eq!(fog_volume.density_factor, 1.0);
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Visible));
    assert_eq!(
        entity_ref.get::<Transform>().unwrap().scale,
        Vec3::new(2.0, 3.0, 4.0)
    );
    assert_eq!(
        entity_ref.get::<Name>().unwrap().as_str(),
        "FogVolumeComponent"
    );
}

#[test]
fn plugin_syncs_lens_flare_component_to_preview_mesh() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(LensFlareComponent {
            configuration: LensFlareConfiguration {
                tint: Color::srgba(0.9, 0.8, 0.1, 1.0),
                brightness: 2.0,
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<Mesh3d>());
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Visible));
    assert!(entity_ref.contains::<Transform>());
    assert_eq!(
        entity_ref.get::<Name>().unwrap().as_str(),
        "LensFlareComponent"
    );
}

#[test]
fn plugin_syncs_particle_component_to_preview_mesh() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(ParticleComponent {
            settings: ParticleEmitterSettings {
                selected_emitter: "fx/fire/sparks".to_string(),
                ..Default::default()
            },
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<Mesh3d>());
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Visible));
    assert!(entity_ref.contains::<Transform>());
    assert_eq!(
        entity_ref.get::<Name>().unwrap().as_str(),
        "ParticleComponent"
    );
}

#[test]
fn plugin_syncs_decal_component_to_preview_mesh() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(DecalComponent {
            configuration: DecalConfiguration {
                color: Color::srgba(0.7, 0.2, 0.1, 1.0),
                opacity: 0.5,
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<Mesh3d>());
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Visible));
    assert!(entity_ref.contains::<Transform>());
    assert_eq!(entity_ref.get::<Name>().unwrap().as_str(), "DecalComponent");
}

#[test]
fn plugin_binds_decal_material_asset_path() {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        AssetPlugin::default(),
        LmbrCentralAssetPlugin,
        LmbrCentralPlugin,
    ))
    .init_resource::<Assets<Mesh>>()
    .init_resource::<Assets<StandardMaterial>>();

    let entity = app
        .world_mut()
        .spawn(DecalComponent {
            configuration: DecalConfiguration {
                material_asset_path: Some("materials/decals/mud.mtl".to_string()),
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    let binding = entity_ref.get::<MaterialAssetBinding>().unwrap();
    assert_eq!(binding.path(), "materials/decals/mud.mtl");
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn plugin_syncs_light_component_to_point_light() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(LightComponent {
            configuration: LightConfiguration {
                point_max_distance: 12.0,
                diffuse_multiplier: 2.0,
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    let light = entity_ref.get::<PointLight>().unwrap();
    assert_eq!(light.range, 12.0);
    assert_eq!(light.intensity, 200_000.0);
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Visible));
    assert!(entity_ref.contains::<Transform>());
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn plugin_syncs_projector_light_to_spot_light() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(LightComponent {
            configuration: LightConfiguration {
                light_type: LightType::Projector,
                projector_range: 18.0,
                projector_fov_degrees: 60.0,
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    let light = entity_ref.get::<SpotLight>().unwrap();
    assert_eq!(light.range, 18.0);
    assert!((light.outer_angle - 30.0_f32.to_radians()).abs() < f32::EPSILON);
    assert!(!entity_ref.contains::<PointLight>());
}

#[test]
fn plugin_syncs_probe_light_to_bevy_light_probe() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(LightComponent {
            configuration: LightConfiguration {
                light_type: LightType::Probe,
                probe_area: Vec3::new(4.0, 5.0, 6.0),
                ..Default::default()
            },
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<BevyLightProbe>());
    assert!(!entity_ref.contains::<PointLight>());
    assert!(!entity_ref.contains::<SpotLight>());
    assert_eq!(
        entity_ref.get::<Transform>().unwrap().scale,
        Vec3::new(4.0, 5.0, 6.0)
    );
    assert_eq!(entity_ref.get::<Visibility>(), Some(&Visibility::Visible));
}
