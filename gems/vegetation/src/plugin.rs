use bevy::prelude::*;

use crate::area::register_area_components;
use crate::audio::register_audio_components;
use crate::descriptor::register_descriptor_components;
use crate::instance::register_instance_components;
use crate::modifiers::register_modifier_components;
use crate::physics::register_physics_components;
use crate::render::register_render_components;
use crate::surface::register_surface_components;

/// Register Vegetation components and systems.
pub struct VegetationPlugin;

impl Plugin for VegetationPlugin {
    fn build(&self, app: &mut App) {
        register_audio_components(app);
        register_surface_components(app);
        register_modifier_components(app);
        register_descriptor_components(app);
        register_instance_components(app);
        register_area_components(app);
        register_physics_components(app);
        register_render_components(app);
    }
}
