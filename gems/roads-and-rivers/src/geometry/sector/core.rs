use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Lumberyard `SplineGeometrySector`: a single 4-point strip segment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct SplineGeometrySector {
    pub points: [Vec3; 4],
    pub t0: f32,
    pub t1: f32,
}
