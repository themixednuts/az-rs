//! Spline geometry component data.

use az_gem_lmbr_central::{EngineSpec, SplineData};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::sector::{SplineGeometrySector, build_spline_geometry_sectors};
use super::width::SplineGeometryWidthModifier;

const MIN_SEGMENT_LENGTH: f32 = 0.5;
const MAX_SEGMENT_LENGTH: f32 = 10.0;

/// Shared Lumberyard `SplineGeometry` data.
#[derive(Component, Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize, Deserialize)]
pub struct SplineGeometry {
    /// Lumberyard field `Width`.
    pub width: SplineGeometryWidthModifier,
    /// Lumberyard field `SegmentLength`.
    pub segment_length: f32,
    /// Lumberyard field `TileLength`.
    pub tile_length: f32,
    /// Lumberyard field `SortPriority`.
    pub sort_priority: i32,
    /// Lumberyard field `ViewDistanceMultiplier`.
    pub view_distance_multiplier: f32,
    /// Lumberyard field `MinSpec`.
    pub min_spec: EngineSpec,
}

impl Default for SplineGeometry {
    fn default() -> Self {
        Self {
            width: SplineGeometryWidthModifier::default(),
            segment_length: 2.0,
            tile_length: 10.0,
            sort_priority: 0,
            view_distance_multiplier: 1.0,
            min_spec: EngineSpec::Low,
        }
    }
}

impl SplineGeometry {
    pub const fn set_segment_length(&mut self, segment_length: f32) {
        self.segment_length = segment_length.clamp(MIN_SEGMENT_LENGTH, MAX_SEGMENT_LENGTH);
    }

    #[must_use]
    pub fn sectors(&self, spline: &SplineData) -> Vec<SplineGeometrySector> {
        build_spline_geometry_sectors(self, spline)
    }
}
