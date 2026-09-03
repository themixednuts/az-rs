use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::shape::bounds::aabb_from_vec3_points;

/// Shared vertex data for Lumberyard spline implementations.
///
/// O3DE reference: `Code/Framework/AzCore/AzCore/Math/Spline.cpp:547`.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct SplineData {
    pub vertices: Vec<Vec3>,
    pub closed: bool,
}

impl SplineData {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.vertices.len()
    }

    #[must_use]
    pub const fn segment_count(&self) -> usize {
        match (self.vertices.len(), self.closed) {
            (0 | 1, _) => 0,
            (len, true) => len,
            (len, false) => len - 1,
        }
    }

    #[must_use]
    pub fn local_bounds(&self) -> Option<Aabb3d> {
        aabb_from_vec3_points(&self.vertices)
    }
}
