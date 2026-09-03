use bevy::prelude::*;

use super::AudioRequest;

/// Audio requests observed by the null backend.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct RecordedAudioRequests {
    pub(super) requests: Vec<AudioRequest>,
}

impl RecordedAudioRequests {
    pub fn iter(&self) -> impl Iterator<Item = &AudioRequest> {
        self.requests.iter()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.requests.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }

    pub fn clear(&mut self) {
        self.requests.clear();
    }

    pub fn drain(&mut self) -> impl Iterator<Item = AudioRequest> + '_ {
        self.requests.drain(..)
    }
}
