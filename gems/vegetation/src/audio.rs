//! Vegetation audio component data.

use bevy::prelude::*;
use uuid::{Uuid, uuid};

/// `VegetationAudioComponent` type ID.
pub const VEGETATION_AUDIO_COMPONENT_TYPE_ID: Uuid = uuid!("DE8B6DD8-3D34-4AF9-A0B3-8FCCCA1AD533");

/// Marker component for vegetation audio processing.
#[derive(Component, Debug, Default, Clone, Copy, PartialEq, Eq, Reflect)]
#[reflect(Component)]
pub struct VegetationAudioComponent;

pub fn register_audio_components(app: &mut App) {
    app.register_type::<VegetationAudioComponent>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vegetation_audio_component_type_id_matches_native_registration() {
        assert_eq!(
            VEGETATION_AUDIO_COMPONENT_TYPE_ID,
            uuid!("DE8B6DD8-3D34-4AF9-A0B3-8FCCCA1AD533")
        );
    }
}
