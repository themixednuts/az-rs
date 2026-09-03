use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::super::data::SplineData;

/// Runtime linear spline data.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:800`.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct LinearSpline {
    pub spline: SplineData,
}
