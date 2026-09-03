use az_gem_audio_system::AudioRequest;
use bevy::prelude::*;

use super::binding::{AudioPreloadBinding, AudioPreloadBindings};
use super::component::AudioPreloadComponent;
use super::load_type::AudioPreloadLoadType;

pub(super) fn sync_audio_preload_components(
    query: Query<(Entity, Ref<AudioPreloadComponent>), Changed<AudioPreloadComponent>>,
    mut bindings: ResMut<AudioPreloadBindings>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for (entity, preload) in &query {
        let desired = preload_binding(preload.as_ref());
        let current = bindings.current(entity);
        if current.as_ref() == desired.as_ref() {
            continue;
        }

        if let Some(current) = current {
            write_unload_preload_request(entity, &current, &mut audio_requests);
        }

        if let Some(desired) = desired {
            write_load_preload_request(entity, &desired, &mut audio_requests);
            bindings.insert(entity, desired);
        } else {
            bindings.remove(entity);
        }
    }
}

pub(super) fn cleanup_removed_audio_preload_components(
    mut removed: RemovedComponents<AudioPreloadComponent>,
    mut bindings: ResMut<AudioPreloadBindings>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for entity in removed.read() {
        if let Some(binding) = bindings.remove(entity) {
            write_unload_preload_request(entity, &binding, &mut audio_requests);
        }
    }
}

fn preload_binding(preload: &AudioPreloadComponent) -> Option<AudioPreloadBinding> {
    if preload.load_type != AudioPreloadLoadType::Auto {
        return None;
    }

    preload
        .default_preload()
        .map(|(name, preload_id)| AudioPreloadBinding::new(preload_id, name))
}

fn write_load_preload_request(
    entity: Entity,
    binding: &AudioPreloadBinding,
    audio_requests: &mut MessageWriter<AudioRequest>,
) {
    audio_requests.write(AudioRequest::LoadPreload {
        entity,
        preload_id: binding.preload_id,
        preload_name: binding.preload_name.clone(),
    });
}

fn write_unload_preload_request(
    entity: Entity,
    binding: &AudioPreloadBinding,
    audio_requests: &mut MessageWriter<AudioRequest>,
) {
    audio_requests.write(AudioRequest::UnloadPreload {
        entity,
        preload_id: binding.preload_id,
        preload_name: binding.preload_name.clone(),
    });
}
