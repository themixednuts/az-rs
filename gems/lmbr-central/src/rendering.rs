//! Rendering components and Bevy sync systems for `LmbrCentral`.

mod common;
mod decal;
mod fog;
mod lens_flare;
mod light;
mod particle;
mod shadow;

use bevy::prelude::*;

use decal::register_decal_components;
use fog::register_fog_volume_components;
use lens_flare::register_lens_flare_components;
use light::register_light_components;
use particle::register_particle_components;
use shadow::register_high_quality_shadow_components;

pub use common::*;
pub use decal::*;
pub use fog::*;
pub use lens_flare::*;
pub use light::*;
pub use particle::*;
pub use shadow::*;

pub fn register_rendering_components(app: &mut App) {
    app.register_type::<EngineSpec>();
    register_fog_volume_components(app);
    register_decal_components(app);
    register_lens_flare_components(app);
    register_particle_components(app);
    register_high_quality_shadow_components(app);
    register_light_components(app);
}

#[cfg(test)]
mod tests;
