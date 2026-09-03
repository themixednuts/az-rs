use az_core::crc::Crc32;
use bevy::prelude::*;

use super::*;

#[test]
fn audio_control_id_matches_lumberyard_crc32_control_hash() {
    let control = AudioControlId::from_name("Play_River_Waterfall");

    assert_eq!(
        control,
        AudioControlId(u64::from(
            Crc32::from_str_lower("Play_River_Waterfall").value()
        ))
    );
    assert!(control.is_valid());
    assert_eq!(AudioControlId::from_name(" "), INVALID_AUDIO_CONTROL_ID);
}

#[test]
fn audio_obstruction_type_maps_native_values() {
    assert_eq!(
        AudioObstructionType::from_native_value(0),
        Some(AudioObstructionType::Ignore)
    );
    assert_eq!(
        AudioObstructionType::from_native_value(1),
        Some(AudioObstructionType::SingleRay)
    );
    assert_eq!(
        AudioObstructionType::from_native_value(2),
        Some(AudioObstructionType::MultiRay)
    );
    assert_eq!(
        AudioObstructionType::from_native_value(3),
        Some(AudioObstructionType::ScatterRaySmall)
    );
    assert_eq!(
        AudioObstructionType::from_native_value(4),
        Some(AudioObstructionType::ScatterRayLarge)
    );
    assert_eq!(
        AudioObstructionType::from_native_value(5),
        Some(AudioObstructionType::None)
    );
    assert_eq!(
        AudioObstructionType::from_native_value(6),
        Some(AudioObstructionType::UseLinkedProxy)
    );
    assert_eq!(AudioObstructionType::from_native_value(7), None);
    assert_eq!(AudioObstructionType::ScatterRayLarge.native_value(), 4);
    assert_eq!(
        AudioObstructionType::UseLinkedProxy.native_name(),
        "eAOOCT_USE_LINKED_PROXY"
    );
}

#[test]
fn plugin_records_audio_requests() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(AudioSystemPlugin);

    let entity = app.world_mut().spawn_empty().id();
    app.world_mut()
        .resource_mut::<Messages<AudioRequest>>()
        .write(AudioRequest::SetMovesWithEntity {
            entity,
            tracks_entity_position: true,
        });

    app.update();

    let requests = app.world().resource::<RecordedAudioRequests>();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests.iter().collect::<Vec<_>>(),
        vec![&AudioRequest::SetMovesWithEntity {
            entity,
            tracks_entity_position: true,
        }]
    );
}
