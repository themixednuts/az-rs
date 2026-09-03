use bevy::prelude::*;

use super::{AudioRequest, RecordedAudioRequests};

pub fn record_audio_requests(
    mut reader: MessageReader<AudioRequest>,
    mut recorded: ResMut<RecordedAudioRequests>,
) {
    recorded.requests.extend(reader.read().cloned());
}
