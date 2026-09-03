//! Audio preload component and lifecycle routing.

use bevy::prelude::*;

mod binding;
mod component;
mod load_type;
mod systems;
mod type_ids;

use systems::{cleanup_removed_audio_preload_components, sync_audio_preload_components};

pub use binding::{AudioPreloadBinding, AudioPreloadBindings};
pub use component::AudioPreloadComponent;
pub use load_type::AudioPreloadLoadType;
pub use type_ids::AUDIO_PRELOAD_COMPONENT_TYPE_ID;

pub(super) fn register_audio_preload_components(app: &mut App) {
    app.init_resource::<AudioPreloadBindings>()
        .register_type::<AudioPreloadLoadType>()
        .register_type::<AudioPreloadBinding>()
        .register_type::<AudioPreloadComponent>()
        .add_systems(
            Update,
            (
                cleanup_removed_audio_preload_components,
                sync_audio_preload_components,
            ),
        );
}
