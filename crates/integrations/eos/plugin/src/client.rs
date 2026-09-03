//! Base EOS client platform lifecycle without anti-cheat.

use crate::{EosService, EosSettings};
use bevy::prelude::*;
use tracing::{error, info};

/// Bevy plugin that owns an EOS client platform and ticks it on the main thread.
pub struct EosClientPlugin;

impl Plugin for EosClientPlugin {
    fn build(&self, app: &mut App) {
        logging_registry::register!();
        app.add_systems(Startup, init_eos_client_service);
        app.add_systems(Update, tick_eos_platform);
    }
}

fn init_eos_client_service(world: &mut World) {
    let settings = match world
        .get_resource::<EosSettings>()
        .cloned()
        .map_or_else(EosSettings::from_env, Ok)
    {
        Ok(settings) => settings,
        Err(error) => {
            error!(%error, "failed to resolve EOS client settings");
            return;
        }
    };

    match EosService::new(&settings) {
        Ok(service) => {
            world.insert_non_send(service);
            info!("EOS client platform initialized");
        }
        Err(error) => error!(%error, "failed to initialize EOS client platform"),
    }
}

fn tick_eos_platform(eos: Option<NonSend<EosService>>) {
    if let Some(eos) = eos {
        eos.tick();
    }
}
