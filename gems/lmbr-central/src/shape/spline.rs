//! Spline shape component data.

mod component;
mod constants;
mod data;
mod variants;

pub use component::{SplineCommon, SplineComponent};
pub use constants::*;
pub use data::SplineData;
pub use variants::{BezierData, BezierSpline, CatmullRomSpline, LinearSpline, Spline, SplineType};
