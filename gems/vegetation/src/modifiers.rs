//! Vegetation filters and modifiers.

mod distribution;
mod position;
mod rotation;
mod scale;
mod slope;
mod stage;
mod surface_altitude;
mod surface_mask;
mod surface_slope;
mod type_ids;

use bevy::prelude::*;

pub use distribution::*;
pub use position::*;
pub use rotation::*;
pub use scale::*;
pub use slope::*;
pub use stage::*;
pub use surface_altitude::*;
pub use surface_mask::*;
pub use surface_slope::*;
pub use type_ids::*;

pub fn register_modifier_components(app: &mut App) {
    app.register_type::<FilterStage>()
        .register_type::<ModifierStage>()
        .register_type::<DistributionFilterConfig>()
        .register_type::<DistributionFilterComponent>()
        .register_type::<SurfaceAltitudeFilterConfig>()
        .register_type::<SurfaceAltitudeFilterComponent>()
        .register_type::<SurfaceMaskFilterConfig>()
        .register_type::<SurfaceMaskFilterComponent>()
        .register_type::<SurfaceSlopeFilterConfig>()
        .register_type::<SurfaceSlopeFilterComponent>()
        .register_type::<PositionModifierConfig>()
        .register_type::<PositionModifierComponent>()
        .register_type::<RotationModifierConfig>()
        .register_type::<RotationModifierComponent>()
        .register_type::<ScaleModifierConfig>()
        .register_type::<ScaleModifierComponent>()
        .register_type::<SlopeAlignmentModifierConfig>()
        .register_type::<SlopeAlignmentModifierComponent>();
}

#[cfg(test)]
mod tests;
