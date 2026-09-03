use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::super::constants::{MAX_SPLINE_GRANULARITY, MIN_SPLINE_GRANULARITY};
use super::super::data::SplineData;

/// Runtime Catmull-Rom spline data.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:1540`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct CatmullRomSpline {
    pub spline: SplineData,
    pub knot_parameterization: f32,
    pub granularity: u16,
}

impl Default for CatmullRomSpline {
    fn default() -> Self {
        Self {
            spline: SplineData::default(),
            knot_parameterization: 0.0,
            granularity: 8,
        }
    }
}

impl CatmullRomSpline {
    #[must_use]
    pub const fn clamped_knot_parameterization(&self) -> f32 {
        self.knot_parameterization.clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn clamped_granularity(&self) -> u16 {
        self.granularity
            .clamp(MIN_SPLINE_GRANULARITY, MAX_SPLINE_GRANULARITY)
    }
}
