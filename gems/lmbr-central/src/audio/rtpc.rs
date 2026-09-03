//! Audio RTPC component and value routing.

use az_gem_audio_system::{AudioControlId, AudioRequest};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

use super::audio_control;

/// Lumberyard `LmbrCentral::AudioRtpcComponent` AZ component UUID.
pub const AUDIO_RTPC_COMPONENT_TYPE_ID: Uuid = uuid!("C54C7AE6-08AA-49E0-B6CD-E1BBB4950DAF");

/// Deprecated Lumberyard `LmbrCentral::AudioRtpcComponent` UUID.
pub const DEPRECATED_AUDIO_RTPC_COMPONENT_TYPE_ID: Uuid =
    uuid!("4441F9C5-D4AD-40CF-B1DE-D5A296C03798");

/// Runtime audio RTPC component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioRtpcComponent.h:24`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioRtpcComponent", version = 1)]
pub struct AudioRtpcComponent {
    pub default_rtpc_name: Option<String>,
}

impl AudioRtpcComponent {
    #[must_use]
    pub fn default_rtpc(&self) -> Option<(&str, AudioControlId)> {
        audio_control(self.default_rtpc_name.as_deref())
    }
}

/// `LmbrCentral` request to set an RTPC value.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioRtpcComponent.h:27`.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct SetAudioRtpcValue {
    pub entity: Entity,
    pub rtpc_name: Option<String>,
    pub value: f32,
}

impl SetAudioRtpcValue {
    #[must_use]
    pub const fn default_rtpc(entity: Entity, value: f32) -> Self {
        Self {
            entity,
            rtpc_name: None,
            value,
        }
    }

    pub fn named(entity: Entity, rtpc_name: impl Into<String>, value: f32) -> Self {
        Self {
            entity,
            rtpc_name: Some(rtpc_name.into()),
            value,
        }
    }
}

fn route_audio_rtpc_value_requests(
    mut requests: MessageReader<SetAudioRtpcValue>,
    rtpcs: Query<&AudioRtpcComponent>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for request in requests.read() {
        let control = request.rtpc_name.as_deref().map_or_else(
            || {
                rtpcs
                    .get(request.entity)
                    .ok()
                    .and_then(AudioRtpcComponent::default_rtpc)
            },
            |rtpc_name| audio_control(Some(rtpc_name)),
        );

        if let Some((rtpc_name, rtpc_id)) = control {
            audio_requests.write(AudioRequest::SetRtpcValue {
                entity: request.entity,
                rtpc_id,
                rtpc_name: rtpc_name.to_string(),
                value: request.value,
            });
        }
    }
}

pub(super) fn register_audio_rtpc_components(app: &mut App) {
    app.register_type::<AudioRtpcComponent>()
        .add_message::<SetAudioRtpcValue>()
        .add_systems(Update, route_audio_rtpc_value_requests);
}
