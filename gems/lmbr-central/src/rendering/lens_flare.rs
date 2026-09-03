//! Lens flare component data and Bevy preview sync.

use bevy::prelude::*;

mod component;
mod config;
mod systems;
mod type_ids;

use systems::sync_lens_flare_components;

pub use component::LensFlareComponent;
pub use config::LensFlareConfiguration;
pub use type_ids::{LENS_FLARE_COMPONENT_TYPE_ID, LENS_FLARE_CONFIGURATION_TYPE_ID};

pub(super) fn register_lens_flare_components(app: &mut App) {
    app.register_type::<LensFlareConfiguration>()
        .register_type::<LensFlareComponent>()
        .add_systems(Update, sync_lens_flare_components);
}
