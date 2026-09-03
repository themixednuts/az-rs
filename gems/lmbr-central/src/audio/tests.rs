use super::*;
use crate::LmbrCentralPlugin;
use az_gem_audio_system::{AudioControlId, AudioObstructionType, AudioRequest};
use bevy::audio::SpatialListener;

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn audio_components_defaults_match_lumberyard_runtime_defaults() {
    let area_environment = AudioAreaEnvironmentComponent::default();
    let environment = AudioEnvironmentComponent::default();
    let preload = AudioPreloadComponent::default();
    let listener = AudioListenerComponent::default();
    let proxy = AudioProxyComponent::default();
    let trigger = AudioTriggerComponent::default();
    let rtpc = AudioRtpcComponent::default();
    let switch = AudioSwitchComponent::default();

    assert_eq!(area_environment.broad_phase_trigger_area, None);
    assert_eq!(area_environment.environment(), None);
    assert_eq!(area_environment.environment_fade_distance, 1.0);

    assert_eq!(environment.default_environment(), None);

    assert_eq!(preload.default_preload(), None);
    assert_eq!(preload.load_type, AudioPreloadLoadType::Auto);

    assert_eq!(listener.rotation_entity_id, 0);
    assert_eq!(listener.position_entity_id, 0);
    assert_eq!(listener.fixed_offset, Vec3::ZERO);
    assert_eq!(listener.offset_ratio, 0.0);
    assert_eq!(listener.listener_transform(None).translation, Vec3::ZERO);

    assert_eq!(proxy.transform_tolerance, 0.0);
    assert_eq!(proxy.occlusion_ignore_radius, 0.0);
    assert!(!proxy.occlusion_ignore_entity);
    assert!(!proxy.occlusion_ignore_entire_entity);
    assert!(!proxy.continuous_bone_update);
    assert!(proxy.tracks_entity_position);

    assert_eq!(trigger.default_play_trigger(), None);
    assert_eq!(trigger.default_stop_trigger(), None);
    assert_eq!(trigger.obstruction_type, AudioObstructionType::Ignore);
    assert!(!trigger.plays_immediately);
    assert!(!trigger.notify_when_trigger_finishes);
    assert!(!trigger.variation_component_linked);
    assert!(!trigger.audio_plays_out_on_deactivate);
    assert!(!trigger.unload_preload_on_completion);

    assert_eq!(rtpc.default_rtpc(), None);
    assert_eq!(switch.default_switch_state(), None);
}

#[test]
#[allow(
    clippy::float_cmp,
    reason = "each assertion pins a value the code under test propagates verbatim - a shipping \
              default, or the exact input this test supplied - so an epsilon compare would \
              let a wrong-but-close value pass"
)]
fn audio_area_environment_amount_at_distance_matches_linear_fade() {
    let area_environment = AudioAreaEnvironmentComponent {
        environment_fade_distance: 4.0,
        ..Default::default()
    };

    assert_eq!(area_environment.amount_at_distance(-1.0), 1.0);
    assert_eq!(area_environment.amount_at_distance(0.0), 1.0);
    assert_eq!(area_environment.amount_at_distance(2.0), 0.5);
    assert_eq!(area_environment.amount_at_distance(4.0), 0.0);
    assert_eq!(area_environment.amount_at_distance(9.0), 0.0);

    let hard_edge = AudioAreaEnvironmentComponent {
        environment_fade_distance: 0.0,
        ..Default::default()
    };
    assert_eq!(hard_edge.amount_at_distance(0.0), 1.0);
    assert_eq!(hard_edge.amount_at_distance(0.1), 0.0);
}

#[test]
fn audio_preload_load_type_maps_native_values() {
    assert_eq!(
        AudioPreloadLoadType::from_native_value(0),
        Some(AudioPreloadLoadType::Auto)
    );
    assert_eq!(
        AudioPreloadLoadType::from_native_value(1),
        Some(AudioPreloadLoadType::Manual)
    );
    assert_eq!(AudioPreloadLoadType::from_native_value(2), None);
    assert_eq!(AudioPreloadLoadType::Manual.native_value(), 1);
}

#[test]
fn plugin_syncs_audio_listener_to_bevy_spatial_listener() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioListenerComponent {
            fixed_offset: Vec3::new(1.0, 2.0, 3.0),
            ..Default::default()
        })
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<SpatialListener>());
    assert_eq!(
        entity_ref.get::<Transform>().unwrap().translation,
        Vec3::new(1.0, 2.0, 3.0)
    );

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![
            AudioRequest::SetListenerEnabled {
                entity,
                enabled: true,
            },
            AudioRequest::SetListenerTransform {
                entity,
                transform: Transform::from_translation(Vec3::new(1.0, 2.0, 3.0)),
            },
        ]
    );
}

#[test]
fn plugin_syncs_audio_proxy_transform_and_tracking_request() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app.world_mut().spawn(AudioProxyComponent::default()).id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<Transform>());

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::SetMovesWithEntity {
            entity,
            tracks_entity_position: true,
        }]
    );
}

#[test]
fn plugin_emits_auto_audio_preload_request() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioPreloadComponent {
            default_preload_name: Some("preload_river_waterfall".to_string()),
            load_type: AudioPreloadLoadType::Auto,
        })
        .id();

    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::LoadPreload {
            entity,
            preload_id: AudioControlId::from_name("preload_river_waterfall"),
            preload_name: "preload_river_waterfall".to_string(),
        }]
    );
    assert_eq!(
        app.world()
            .resource::<AudioPreloadBindings>()
            .binding(entity),
        Some(&AudioPreloadBinding::new(
            AudioControlId::from_name("preload_river_waterfall"),
            "preload_river_waterfall",
        ))
    );
}

#[test]
fn plugin_replaces_auto_audio_preload_request_when_name_changes() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioPreloadComponent {
            default_preload_name: Some("preload_river_waterfall".to_string()),
            load_type: AudioPreloadLoadType::Auto,
        })
        .id();

    app.update();
    app.world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .for_each(drop);

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<AudioPreloadComponent>()
        .unwrap()
        .default_preload_name = Some("preload_forest_ambience".to_string());
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![
            AudioRequest::UnloadPreload {
                entity,
                preload_id: AudioControlId::from_name("preload_river_waterfall"),
                preload_name: "preload_river_waterfall".to_string(),
            },
            AudioRequest::LoadPreload {
                entity,
                preload_id: AudioControlId::from_name("preload_forest_ambience"),
                preload_name: "preload_forest_ambience".to_string(),
            },
        ]
    );
}

#[test]
fn plugin_unloads_auto_audio_preload_when_component_goes_manual_or_is_removed() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioPreloadComponent {
            default_preload_name: Some("preload_river_waterfall".to_string()),
            load_type: AudioPreloadLoadType::Auto,
        })
        .id();

    app.update();
    app.world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .for_each(drop);

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<AudioPreloadComponent>()
        .unwrap()
        .load_type = AudioPreloadLoadType::Manual;
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::UnloadPreload {
            entity,
            preload_id: AudioControlId::from_name("preload_river_waterfall"),
            preload_name: "preload_river_waterfall".to_string(),
        }]
    );
    assert!(app.world().resource::<AudioPreloadBindings>().is_empty());

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<AudioPreloadComponent>()
        .unwrap()
        .load_type = AudioPreloadLoadType::Auto;
    app.update();
    app.world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .for_each(drop);

    app.world_mut()
        .entity_mut(entity)
        .remove::<AudioPreloadComponent>();
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::UnloadPreload {
            entity,
            preload_id: AudioControlId::from_name("preload_river_waterfall"),
            preload_name: "preload_river_waterfall".to_string(),
        }]
    );
    assert!(app.world().resource::<AudioPreloadBindings>().is_empty());
}

#[test]
fn plugin_emits_audio_switch_state_request() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioSwitchComponent {
            default_switch_name: Some("SurfaceMaterial".to_string()),
            default_state_name: Some("Snow".to_string()),
        })
        .id();

    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::SetSwitchState {
            entity,
            switch_id: AudioControlId::from_name("SurfaceMaterial"),
            switch_name: "SurfaceMaterial".to_string(),
            state_id: AudioControlId::from_name("Snow"),
            state_name: "Snow".to_string(),
        }]
    );
}

#[test]
fn plugin_routes_default_audio_environment_amount_request() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioEnvironmentComponent {
            default_environment_name: Some("CaveReverb".to_string()),
        })
        .id();

    app.world_mut()
        .resource_mut::<Messages<SetAudioEnvironmentAmount>>()
        .write(SetAudioEnvironmentAmount::default_environment(entity, 0.75));
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::SetEnvironmentAmount {
            entity,
            environment_id: AudioControlId::from_name("CaveReverb"),
            environment_name: "CaveReverb".to_string(),
            amount: 0.75,
        }]
    );
}

#[test]
fn plugin_routes_audio_area_environment_amount_request_to_audio_entity() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let area_entity = app
        .world_mut()
        .spawn(AudioAreaEnvironmentComponent {
            environment_name: Some("DungeonInterior".to_string()),
            ..Default::default()
        })
        .id();
    let audio_entity = app.world_mut().spawn_empty().id();

    app.world_mut()
        .resource_mut::<Messages<SetAudioAreaEnvironmentAmount>>()
        .write(SetAudioAreaEnvironmentAmount::new(
            area_entity,
            audio_entity,
            0.25,
        ));
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::SetEnvironmentAmount {
            entity: audio_entity,
            environment_id: AudioControlId::from_name("DungeonInterior"),
            environment_name: "DungeonInterior".to_string(),
            amount: 0.25,
        }]
    );
}

#[test]
fn plugin_routes_named_audio_rtpc_value_request() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app.world_mut().spawn_empty().id();

    app.world_mut()
        .resource_mut::<Messages<SetAudioRtpcValue>>()
        .write(SetAudioRtpcValue::named(entity, "MusicIntensity", 23.5));
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::SetRtpcValue {
            entity,
            rtpc_id: AudioControlId::from_name("MusicIntensity"),
            rtpc_name: "MusicIntensity".to_string(),
            value: 23.5,
        }]
    );
}

#[test]
fn plugin_emits_immediate_audio_trigger_request() {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .add_plugins(LmbrCentralPlugin);

    let entity = app
        .world_mut()
        .spawn(AudioTriggerComponent {
            default_play_trigger_name: Some("Play_River_Waterfall".to_string()),
            obstruction_type: AudioObstructionType::SingleRay,
            plays_immediately: true,
            notify_when_trigger_finishes: true,
            ..Default::default()
        })
        .id();

    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();

    assert_eq!(
        messages,
        vec![
            AudioRequest::SetObstructionType {
                entity,
                obstruction_type: AudioObstructionType::SingleRay,
            },
            AudioRequest::ExecuteTrigger {
                entity,
                trigger_id: AudioControlId::from_name("Play_River_Waterfall"),
                trigger_name: "Play_River_Waterfall".to_string(),
                notify_when_finished: true,
            },
        ]
    );

    app.world_mut()
        .entity_mut(entity)
        .get_mut::<AudioTriggerComponent>()
        .unwrap()
        .obstruction_type = AudioObstructionType::MultiRay;
    app.update();

    let messages = app
        .world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .drain()
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![AudioRequest::SetObstructionType {
            entity,
            obstruction_type: AudioObstructionType::MultiRay,
        }]
    );
}
