//! Audio listener component and Bevy listener sync.

use az_gem_audio_system::AudioRequest;
use bevy::audio::SpatialListener;
use bevy::prelude::*;
use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::AudioListenerComponent` AZ component UUID.
pub const AUDIO_LISTENER_COMPONENT_TYPE_ID: Uuid = uuid!("00B5358C-3EEE-4012-93FC-6222B0004404");

/// Runtime audio listener component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioListenerComponent.h:38`.
#[derive(Component, Debug, Clone, PartialEq, Reflect)]
#[reflect(Component)]
pub struct AudioListenerComponent {
    pub rotation_entity_id: u64,
    pub position_entity_id: u64,
    pub fixed_offset: Vec3,
    pub offset_ratio: f32,
}

impl Default for AudioListenerComponent {
    fn default() -> Self {
        Self {
            rotation_entity_id: 0,
            position_entity_id: 0,
            fixed_offset: Vec3::ZERO,
            offset_ratio: 0.0,
        }
    }
}

impl AudioListenerComponent {
    #[must_use]
    pub fn listener_transform(&self, base_transform: Option<&Transform>) -> Transform {
        let mut transform = base_transform.copied().unwrap_or_default();
        transform.translation += self.fixed_offset;
        transform
    }
}

#[allow(clippy::type_complexity)]
fn sync_audio_listener_components(
    mut commands: Commands,
    query: Query<
        (
            Entity,
            Ref<AudioListenerComponent>,
            Option<&SpatialListener>,
            Option<&Transform>,
        ),
        Or<(
            Changed<AudioListenerComponent>,
            Changed<Transform>,
            Without<SpatialListener>,
        )>,
    >,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for (entity, listener, spatial_listener, transform) in &query {
        let mut entity_commands = commands.entity(entity);
        if spatial_listener.is_none() {
            entity_commands.insert(SpatialListener::default());
        }
        if transform.is_none() {
            entity_commands.insert(listener.listener_transform(None));
        }

        if listener.is_added() {
            audio_requests.write(AudioRequest::SetListenerEnabled {
                entity,
                enabled: true,
            });
        }

        audio_requests.write(AudioRequest::SetListenerTransform {
            entity,
            transform: listener.listener_transform(transform),
        });
    }
}

pub(super) fn register_audio_listener_components(app: &mut App) {
    app.register_type::<AudioListenerComponent>()
        .add_systems(Update, sync_audio_listener_components);
}
