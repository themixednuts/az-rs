use std::collections::HashMap;

use az_gem_audio_system::AudioControlId;
use bevy::prelude::*;

/// Audio preload currently requested by an entity.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioPreloadComponent.cpp:76`.
#[derive(Debug, Clone, PartialEq, Eq, Reflect)]
pub struct AudioPreloadBinding {
    pub preload_id: AudioControlId,
    pub preload_name: String,
}

impl AudioPreloadBinding {
    #[must_use]
    pub fn new(preload_id: AudioControlId, preload_name: impl Into<String>) -> Self {
        Self {
            preload_id,
            preload_name: preload_name.into(),
        }
    }
}

/// Tracks entity preload requests that need matching unloads.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioPreloadBindings {
    loaded: HashMap<Entity, AudioPreloadBinding>,
}

impl AudioPreloadBindings {
    #[must_use]
    pub fn binding(&self, entity: Entity) -> Option<&AudioPreloadBinding> {
        self.loaded.get(&entity)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty()
    }

    pub(super) fn current(&self, entity: Entity) -> Option<AudioPreloadBinding> {
        self.loaded.get(&entity).cloned()
    }

    pub(super) fn insert(&mut self, entity: Entity, binding: AudioPreloadBinding) {
        self.loaded.insert(entity, binding);
    }

    pub(super) fn remove(&mut self, entity: Entity) -> Option<AudioPreloadBinding> {
        self.loaded.remove(&entity)
    }
}
