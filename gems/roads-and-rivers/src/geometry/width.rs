//! Spline width interpolation data.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use super::math::{ease_in_out, inverse_lerp};

/// A keyframe holding a width offset at a distance along the spline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct DistanceWidth {
    pub distance: f32,
    pub width: f32,
}

/// Port of Lumberyard `RoadWidthInterpolator`.
#[derive(Debug, Clone, Default, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct RoadWidthInterpolator {
    distance_widths: Vec<DistanceWidth>,
}

impl RoadWidthInterpolator {
    #[must_use]
    pub fn width(&self, distance: f32) -> f32 {
        let Some(first) = self.distance_widths.first().copied() else {
            return 0.0;
        };

        if distance < first.distance {
            return first.width;
        }

        for pair in self.distance_widths.windows(2) {
            let previous = pair[0];
            let current = pair[1];
            if distance < current.distance {
                let t = inverse_lerp(previous.distance, current.distance, distance);
                return previous.width.lerp(current.width, ease_in_out(t));
            }
        }

        self.distance_widths.last().map_or(0.0, |entry| entry.width)
    }

    pub fn maximum_width(&self) -> f32 {
        self.distance_widths
            .iter()
            .map(|entry| entry.width)
            .reduce(f32::max)
            .unwrap_or(0.0)
    }

    pub fn insert_distance_width_key_frame(&mut self, distance: f32, width: f32) {
        let key = DistanceWidth { distance, width };
        let index = self
            .distance_widths
            .partition_point(|entry| entry.distance <= distance);
        self.distance_widths.insert(index, key);
    }

    pub fn clear(&mut self) {
        self.distance_widths.clear();
    }

    #[must_use]
    pub fn key_frames(&self) -> &[DistanceWidth] {
        &self.distance_widths
    }
}

/// Width data reflected by Lumberyard as `SplineGeometryWidthModifier`.
#[derive(Debug, Clone, PartialEq, Reflect, Serialize, Deserialize)]
#[reflect(Serialize, Deserialize)]
pub struct SplineGeometryWidthModifier {
    /// Lumberyard field `DefaultWidth`.
    pub default_width: f32,
    /// Lumberyard field `WidthAttribute`, indexed by source spline vertex.
    pub width_attribute: Vec<f32>,
}

impl Default for SplineGeometryWidthModifier {
    fn default() -> Self {
        Self {
            default_width: 5.0,
            width_attribute: Vec::new(),
        }
    }
}

impl SplineGeometryWidthModifier {
    #[must_use]
    pub fn width_at(&self, distance: f32, vertex_distances: &[f32]) -> f32 {
        self.default_width + self.width_interpolator(vertex_distances).width(distance)
    }

    #[must_use]
    pub fn maximum_width(&self, vertex_distances: &[f32]) -> f32 {
        self.default_width + self.width_interpolator(vertex_distances).maximum_width()
    }

    fn width_interpolator(&self, vertex_distances: &[f32]) -> RoadWidthInterpolator {
        let mut interpolator = RoadWidthInterpolator::default();
        for (distance, width) in vertex_distances
            .iter()
            .copied()
            .zip(self.width_attribute.iter().copied())
        {
            interpolator.insert_distance_width_key_frame(distance, width);
        }
        interpolator
    }
}
