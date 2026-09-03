use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::super::constants::{MAX_SPLINE_GRANULARITY, MIN_SPLINE_GRANULARITY};
use super::super::data::SplineData;

/// Per-vertex Bezier spline control data.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:1282`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BezierData {
    pub forward: Vec3,
    pub back: Vec3,
    pub angle: f32,
}

/// Runtime Bezier spline data.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:1289`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct BezierSpline {
    pub spline: SplineData,
    pub bezier_data: Vec<BezierData>,
    pub granularity: u16,
}

impl Default for BezierSpline {
    fn default() -> Self {
        Self {
            spline: SplineData::default(),
            bezier_data: Vec::new(),
            granularity: 8,
        }
    }
}

impl BezierSpline {
    #[must_use]
    pub fn clamped_granularity(&self) -> u16 {
        self.granularity
            .clamp(MIN_SPLINE_GRANULARITY, MAX_SPLINE_GRANULARITY)
    }
}
