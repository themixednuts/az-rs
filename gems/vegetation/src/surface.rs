//! Surface tag data for vegetation.

mod constants;
mod distance;
mod tag;
mod weight;

use bevy::prelude::*;

pub use constants::*;
pub use distance::VegetationSurfaceTagDistance;
pub use tag::VegetationSurfaceTag;
pub use weight::{
    VegetationSurfaceTagDepth, VegetationSurfaceTagOffset, VegetationSurfaceTagWeight,
    add_max_surface_weight, has_matching_surface_tag_weight, has_valid_surface_tags,
    merge_max_surface_weights,
};

pub fn register_surface_components(app: &mut App) {
    app.register_type::<VegetationSurfaceTag>()
        .register_type::<VegetationSurfaceTagWeight>()
        .register_type::<VegetationSurfaceTagOffset>()
        .register_type::<VegetationSurfaceTagDepth>()
        .register_type::<VegetationSurfaceTagDistance>();
}

#[cfg(test)]
mod tests;
