//! Audio area environment component and routing.

use az_gem_audio_system::{AudioControlId, AudioRequest};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

use super::audio_control;

/// Lumberyard `LmbrCentral::AudioAreaEnvironmentComponent` AZ component UUID.
pub const AUDIO_AREA_ENVIRONMENT_COMPONENT_TYPE_ID: Uuid =
    uuid!("52300012-FFCD-4559-9479-20F463940320");

/// Runtime audio area environment component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioAreaEnvironmentComponent.h:30`.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioAreaEnvironmentComponent", version = 1)]
pub struct AudioAreaEnvironmentComponent {
    #[entities]
    pub broad_phase_trigger_area: Option<Entity>,
    pub environment_name: Option<String>,
    pub environment_fade_distance: f32,
}

impl Default for AudioAreaEnvironmentComponent {
    fn default() -> Self {
        Self {
            broad_phase_trigger_area: None,
            environment_name: None,
            environment_fade_distance: 1.0,
        }
    }
}

impl AudioAreaEnvironmentComponent {
    #[must_use]
    pub fn environment(&self) -> Option<(&str, AudioControlId)> {
        audio_control(self.environment_name.as_deref())
    }

    #[must_use]
    pub fn amount_at_distance(&self, distance_from_shape: f32) -> f32 {
        let fade_distance = self.environment_fade_distance;
        if !fade_distance.is_finite() || fade_distance <= 0.0 {
            return if distance_from_shape <= 0.0 { 1.0 } else { 0.0 };
        }

        if !distance_from_shape.is_finite() {
            return 0.0;
        }

        let distance = distance_from_shape.clamp(0.0, fade_distance);
        1.0 - distance / fade_distance
    }
}

/// `LmbrCentral` request to set an audio area environment amount.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioAreaEnvironmentComponent.cpp:99`.
#[derive(Message, Debug, Clone, PartialEq)]
pub struct SetAudioAreaEnvironmentAmount {
    pub area_entity: Entity,
    pub audio_entity: Entity,
    pub amount: f32,
}

impl SetAudioAreaEnvironmentAmount {
    #[must_use]
    pub const fn new(area_entity: Entity, audio_entity: Entity, amount: f32) -> Self {
        Self {
            area_entity,
            audio_entity,
            amount,
        }
    }
}

fn route_audio_area_environment_amount_requests(
    mut requests: MessageReader<SetAudioAreaEnvironmentAmount>,
    area_environments: Query<&AudioAreaEnvironmentComponent>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for request in requests.read() {
        let Ok(area_environment) = area_environments.get(request.area_entity) else {
            continue;
        };

        if let Some((environment_name, environment_id)) = area_environment.environment() {
            audio_requests.write(AudioRequest::SetEnvironmentAmount {
                entity: request.audio_entity,
                environment_id,
                environment_name: environment_name.to_string(),
                amount: request.amount,
            });
        }
    }
}

pub(super) fn register_audio_area_environment_components(app: &mut App) {
    app.register_type::<AudioAreaEnvironmentComponent>()
        .add_message::<SetAudioAreaEnvironmentAmount>()
        .add_systems(Update, route_audio_area_environment_amount_requests);
}
