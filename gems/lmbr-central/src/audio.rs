//! Audio components and request routing for `LmbrCentral`.
//!
//! O3DE reference: `Gems/LmbrCentral/Code/Source/Audio/AudioTriggerComponent.h`.

mod area_environment;
mod control;
mod environment;
mod listener;
mod preload;
mod proxy;
mod rtpc;
mod shape;
mod switch;
mod trigger;

use az_gem_audio_system::{AudioControlId, AudioObstructionType, AudioRequest};
use bevy::prelude::*;

use area_environment::register_audio_area_environment_components;
use environment::register_audio_environment_components;
use listener::register_audio_listener_components;
use preload::register_audio_preload_components;
use proxy::register_audio_proxy_components;
use rtpc::register_audio_rtpc_components;
use shape::register_audio_shape_components;
use switch::register_audio_switch_components;
use trigger::register_audio_trigger_components;

use control::audio_control;

pub use area_environment::*;
pub use environment::*;
pub use listener::*;
pub use preload::*;
pub use proxy::*;
pub use rtpc::*;
pub use shape::*;
pub use switch::*;
pub use trigger::*;

pub fn register_audio_components(app: &mut App) {
    app.register_type::<AudioControlId>()
        .register_type::<AudioObstructionType>()
        .add_message::<AudioRequest>();
    register_audio_area_environment_components(app);
    register_audio_environment_components(app);
    register_audio_preload_components(app);
    register_audio_listener_components(app);
    register_audio_proxy_components(app);
    register_audio_trigger_components(app);
    register_audio_rtpc_components(app);
    register_audio_shape_components(app);
    register_audio_switch_components(app);
}

#[cfg(test)]
mod tests;
