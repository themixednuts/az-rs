use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::super::data::SplineData;
use super::{BezierSpline, CatmullRomSpline, LinearSpline, SplineType};

/// Runtime spline implementation.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.h:83`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub enum Spline {
    Linear(LinearSpline),
    Bezier(BezierSpline),
    CatmullRom(CatmullRomSpline),
}

impl Default for Spline {
    fn default() -> Self {
        Self::Linear(LinearSpline::default())
    }
}

impl Spline {
    #[must_use]
    pub const fn spline_type(&self) -> SplineType {
        match self {
            Self::Linear(_) => SplineType::Linear,
            Self::Bezier(_) => SplineType::Bezier,
            Self::CatmullRom(_) => SplineType::CatmullRom,
        }
    }

    #[must_use]
    pub const fn data(&self) -> &SplineData {
        match self {
            Self::Linear(spline) => &spline.spline,
            Self::Bezier(spline) => &spline.spline,
            Self::CatmullRom(spline) => &spline.spline,
        }
    }

    pub const fn data_mut(&mut self) -> &mut SplineData {
        match self {
            Self::Linear(spline) => &mut spline.spline,
            Self::Bezier(spline) => &mut spline.spline,
            Self::CatmullRom(spline) => &mut spline.spline,
        }
    }

    #[must_use]
    pub fn local_bounds(&self) -> Option<Aabb3d> {
        self.data().local_bounds()
    }
}
