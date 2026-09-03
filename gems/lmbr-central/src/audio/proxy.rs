//! Audio proxy component and transform tracking sync.

use az_derive::AzRtti;
use az_gem_audio_system::AudioRequest;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::{Uuid, uuid};

/// Lumberyard `LmbrCentral::AudioProxyComponent` AZ component UUID.
pub const AUDIO_PROXY_COMPONENT_TYPE_ID: Uuid = uuid!("0EE6EE0F-7939-4AB8-B0E3-F9B3925D61EE");

/// Runtime audio proxy component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioProxyComponent.h:26`.
#[derive(AzRtti, Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize, Prefab)]
#[az_rtti(
    name = "LmbrCentral::AudioProxyComponent",
    AUDIO_PROXY_COMPONENT_TYPE_ID,
    register
)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
// The namespace keeps this prefab tag distinct from project-defined audio proxy tags.
#[prefab(tag = "azoth.lmbr_central.AudioProxyComponent", version = 1)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the reflected field set of LmbrCentral::AudioProxyComponent \
              (0EE6EE0F-7939-4AB8-B0E3-F9B3925D61EE, AudioProxyComponent.h:26) and is \
              written field-by-field into the azoth.lmbr_central.AudioProxyComponent \
              prefab document; packing the flags would rename those stored fields"
)]
pub struct AudioProxyComponent {
    pub transform_tolerance: f32,
    pub occlusion_ignore_radius: f32,
    pub occlusion_ignore_entity: bool,
    pub occlusion_ignore_entire_entity: bool,
    pub continuous_bone_update: bool,
    pub tracks_entity_position: bool,
}

impl Default for AudioProxyComponent {
    fn default() -> Self {
        Self {
            transform_tolerance: 0.0,
            occlusion_ignore_radius: 0.0,
            occlusion_ignore_entity: false,
            occlusion_ignore_entire_entity: false,
            continuous_bone_update: false,
            tracks_entity_position: true,
        }
    }
}

#[allow(clippy::type_complexity)]
fn sync_audio_proxy_components(
    mut commands: Commands,
    query: Query<
        (Entity, &AudioProxyComponent, Option<&Transform>),
        Or<(Changed<AudioProxyComponent>, Without<Transform>)>,
    >,
    mut audio_requests: MessageWriter<AudioRequest>,
) {
    for (entity, proxy, transform) in &query {
        if transform.is_none() {
            commands.entity(entity).insert(Transform::default());
        }

        audio_requests.write(AudioRequest::SetMovesWithEntity {
            entity,
            tracks_entity_position: proxy.tracks_entity_position,
        });
    }
}

pub(super) fn register_audio_proxy_components(app: &mut App) {
    app.register_type::<AudioProxyComponent>()
        .add_systems(Update, sync_audio_proxy_components);
}
