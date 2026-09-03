//! Audio switch component and state sync.

use az_gem_audio_system::{AudioControlId, AudioRequest};
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use uuid::{Uuid, uuid};

use super::audio_control;

/// Lumberyard `LmbrCentral::AudioSwitchComponent` AZ component UUID.
pub const AUDIO_SWITCH_COMPONENT_TYPE_ID: Uuid = uuid!("85FD9037-A5EA-4783-B49A-7959BBB34011");

/// Deprecated Lumberyard `LmbrCentral::AudioSwitchComponent` UUID.
pub const DEPRECATED_AUDIO_SWITCH_COMPONENT_TYPE_ID: Uuid =
    uuid!("7A23B947-8EE8-4E6B-A772-C43BBBB4D090");

/// Runtime audio switch component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioSwitchComponent.h:28`.
#[derive(Component, Debug, Clone, Default, PartialEq, Eq, Reflect, Prefab)]
#[reflect(Component, Default, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioSwitchComponent", version = 1)]
pub struct AudioSwitchComponent {
    pub default_switch_name: Option<String>,
    pub default_state_name: Option<String>,
}

impl AudioSwitchComponent {
    #[must_use]
    pub fn default_switch_state(&self) -> Option<(&str, AudioControlId, &str, AudioControlId)> {
        let (switch_name, switch_id) = audio_control(self.default_switch_name.as_deref())?;
        let (state_name, state_id) = audio_control(self.default_state_name.as_deref())?;
        Some((switch_name, switch_id, state_name, state_id))
    }
}

fn sync_audio_switch_components(
    query: Query<(Entity, Ref<AudioSwitchComponent>), Changed<AudioSwitchComponent>>,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for (entity, switch) in &query {
        if let Some((switch_name, switch_id, state_name, state_id)) = switch.default_switch_state()
        {
            audio_requests.write(AudioRequest::SetSwitchState {
                entity,
                switch_id,
                switch_name: switch_name.to_string(),
                state_id,
                state_name: state_name.to_string(),
            });
        }
    }
}

pub(super) fn register_audio_switch_components(app: &mut App) {
    app.register_type::<AudioSwitchComponent>()
        .add_systems(Update, sync_audio_switch_components);
}
