//! Audio environment component and routing.

use az_gem_audio_system::{AudioControlId, AudioRequest};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

use super::audio_control;

/// Lumberyard `LmbrCentral::AudioEnvironmentComponent` AZ component UUID.
pub const AUDIO_ENVIRONMENT_COMPONENT_TYPE_ID: Uuid = uuid!("D5085D04-2522-4585-9E65-D337C5BBB8A7");

/// Runtime audio environment component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioEnvironmentComponent.h:29`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioEnvironmentComponent", version = 1)]
pub struct AudioEnvironmentComponent {
    pub default_environment_name: Option<String>,
}

impl AudioEnvironmentComponent {
    #[must_use]
    pub fn default_environment(&self) -> Option<(&str, AudioControlId)> {
        audio_control(self.default_environment_name.as_deref())
    }
}

/// `LmbrCentral` request to set an audio environment amount.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioEnvironmentComponent.h:32`.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct SetAudioEnvironmentAmount {
    pub entity: Entity,
    pub environment_name: Option<String>,
    pub amount: f32,
}

impl SetAudioEnvironmentAmount {
    #[must_use]
    pub const fn default_environment(entity: Entity, amount: f32) -> Self {
        Self {
            entity,
            environment_name: None,
            amount,
        }
    }

    pub fn named(entity: Entity, environment_name: impl Into<String>, amount: f32) -> Self {
        Self {
            entity,
            environment_name: Some(environment_name.into()),
            amount,
        }
    }
}

fn route_audio_environment_amount_requests(
    mut requests: MessageReader<SetAudioEnvironmentAmount>,
    environments: Query<&AudioEnvironmentComponent>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for request in requests.read() {
        let control = request.environment_name.as_deref().map_or_else(
            || {
                environments
                    .get(request.entity)
                    .ok()
                    .and_then(AudioEnvironmentComponent::default_environment)
            },
            |environment_name| audio_control(Some(environment_name)),
        );

        if let Some((environment_name, environment_id)) = control {
            audio_requests.write(AudioRequest::SetEnvironmentAmount {
                entity: request.entity,
                environment_id,
                environment_name: environment_name.to_string(),
                amount: request.amount,
            });
        }
    }
}

pub(super) fn register_audio_environment_components(app: &mut App) {
    app.register_type::<AudioEnvironmentComponent>()
        .add_message::<SetAudioEnvironmentAmount>()
        .add_systems(Update, route_audio_environment_amount_requests);
}
