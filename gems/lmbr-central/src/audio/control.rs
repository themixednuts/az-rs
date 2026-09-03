//! Shared audio control-name helpers.

use az_gem_audio_system::AudioControlId;

use crate::non_empty_path;

pub(super) fn audio_control(value: Option<&str>) -> Option<(&str, AudioControlId)> {
    non_empty_path(value)
        .map(|name| (name, AudioControlId::from_name(name)))
        .filter(|(_, id)| id.is_valid())
}
