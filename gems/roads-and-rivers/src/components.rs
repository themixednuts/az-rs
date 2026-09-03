//! Road and river component data.

mod ids;
mod river;
mod road;

use bevy::prelude::*;

pub use ids::*;
pub use river::*;
pub use road::*;

pub fn register_road_river_components(app: &mut App) {
    app.register_type::<RoadComponent>()
        .register_type::<RiverComponent>();
}

#[cfg(test)]
mod tests;
