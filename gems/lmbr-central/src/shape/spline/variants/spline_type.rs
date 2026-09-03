use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Spline interpolation type.
///
/// O3DE reference: `Gems/LmbrCentral/Code/Source/Shape/SplineComponent.cpp:23`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum SplineType {
    #[default]
    Linear,
    Bezier,
    CatmullRom,
}
