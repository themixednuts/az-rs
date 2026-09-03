//! Audio trigger component and trigger request sync.

use az_gem_audio_system::{AudioControlId, AudioObstructionType, AudioRequest};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

use super::audio_control;

/// Lumberyard `LmbrCentral::AudioTriggerComponent` AZ component UUID.
pub const AUDIO_TRIGGER_COMPONENT_TYPE_ID: Uuid = uuid!("8CBBB54B-7435-4D33-844D-E7F201BD581A");

/// Deprecated Lumberyard `LmbrCentral::AudioTriggerComponent` UUID.
pub const DEPRECATED_AUDIO_TRIGGER_COMPONENT_TYPE_ID: Uuid =
    uuid!("80089838-4444-4D67-9A89-66B0276BB916");

/// Deprecated Lumberyard `LmbrCentral::AudioComponent` UUID.
pub const DEPRECATED_AUDIO_COMPONENT_TYPE_ID: Uuid = uuid!("53033C2C-EE40-4D19-A7F4-861D6AA820EB");

/// Runtime audio trigger component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioTriggerComponent.h:27`.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioTriggerComponent", version = 1)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the reflected field set of LmbrCentral::AudioTriggerComponent \
              (8CBBB54B-7435-4D33-844D-E7F201BD581A, AudioTriggerComponent.h:27) and is \
              written field-by-field into the azoth.lmbr_central.AudioTriggerComponent \
              prefab document; packing the flags would rename those stored fields"
)]
pub struct AudioTriggerComponent {
    pub default_play_trigger_name: Option<String>,
    pub default_stop_trigger_name: Option<String>,
    pub obstruction_type: AudioObstructionType,
    pub plays_immediately: bool,
    pub notify_when_trigger_finishes: bool,
    pub variation_component_linked: bool,
    pub audio_plays_out_on_deactivate: bool,
    pub unload_preload_on_completion: bool,
}

impl Default for AudioTriggerComponent {
    fn default() -> Self {
        Self {
            default_play_trigger_name: None,
            default_stop_trigger_name: None,
            obstruction_type: AudioObstructionType::Ignore,
            plays_immediately: false,
            notify_when_trigger_finishes: false,
            variation_component_linked: false,
            audio_plays_out_on_deactivate: false,
            unload_preload_on_completion: false,
        }
    }
}

impl AudioTriggerComponent {
    #[must_use]
    pub fn default_play_trigger(&self) -> Option<(&str, AudioControlId)> {
        audio_control(self.default_play_trigger_name.as_deref())
    }

    #[must_use]
    pub fn default_stop_trigger(&self) -> Option<(&str, AudioControlId)> {
        audio_control(self.default_stop_trigger_name.as_deref())
    }
}

fn sync_audio_trigger_components(
    query: Query<(Entity, Ref<AudioTriggerComponent>), Changed<AudioTriggerComponent>>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for (entity, trigger) in &query {
        audio_requests.write(AudioRequest::SetObstructionType {
            entity,
            obstruction_type: trigger.obstruction_type,
        });

        if trigger.is_added()
            && trigger.plays_immediately
            && let Some((name, trigger_id)) = trigger.default_play_trigger()
        {
            audio_requests.write(AudioRequest::ExecuteTrigger {
                entity,
                trigger_id,
                trigger_name: name.to_string(),
                notify_when_finished: trigger.notify_when_trigger_finishes,
            });
        }
    }
}

pub(super) fn register_audio_trigger_components(app: &mut App) {
    app.register_type::<AudioTriggerComponent>()
        .add_systems(Update, sync_audio_trigger_components);
}
