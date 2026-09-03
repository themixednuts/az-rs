use az_gem_audio_system::AudioControlId;
use az_prefab::{Prefab, ReflectPrefab};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::super::audio_control;
use super::load_type::AudioPreloadLoadType;

/// Runtime audio preload component.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioPreloadComponent.h:25`.
#[derive(Component, Debug, Clone, PartialEq, Eq, Reflect, Serialize, Deserialize, Prefab)]
#[reflect(Component, Default, Serialize, Deserialize, Prefab)]
// Azoth prefab-format versioning starts at 1 and only bumps with real migration
// steps once documents ship. Independent of ObjectStream SERIALIZE_VERSION.
#[prefab(tag = "azoth.lmbr_central.AudioPreloadComponent", version = 1)]
pub struct AudioPreloadComponent {
    pub default_preload_name: Option<String>,
    pub load_type: AudioPreloadLoadType,
}

impl Default for AudioPreloadComponent {
    fn default() -> Self {
        Self {
            default_preload_name: None,
            load_type: AudioPreloadLoadType::Auto,
        }
    }
}

impl AudioPreloadComponent {
    #[must_use]
    pub fn default_preload(&self) -> Option<(&str, AudioControlId)> {
        audio_control(self.default_preload_name.as_deref())
    }
}
