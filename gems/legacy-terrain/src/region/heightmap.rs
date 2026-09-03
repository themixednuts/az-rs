//! Terrain region height samples.

use bevy::prelude::*;

use crate::heightmap::math::{ExactF32, clamped_index};
use crate::world::DEFAULT_TERRAIN_UNIT_SIZE_METERS;

/// Raw height samples for one terrain region.
#[derive(Debug, Clone, PartialEq, Reflect)]
pub struct RegionHeightmap {
    pub resolution: usize,
    pub cell_size: f32,
    pub height_scale: f32,
    pub min_height: f32,
    pub max_height: f32,
    pub samples: Vec<u16>,
}

impl Default for RegionHeightmap {
    fn default() -> Self {
        Self {
            resolution: 0,
            cell_size: DEFAULT_TERRAIN_UNIT_SIZE_METERS.exact_f32(),
            height_scale: 1.0,
            min_height: 0.0,
            max_height: 0.0,
            samples: Vec::new(),
        }
    }
}

impl RegionHeightmap {
    /// Build a region heightmap from raw `u16` samples.
    ///
    /// # Errors
    ///
    /// Returns `"height sample count does not match resolution"` if `samples`
    /// does not hold exactly `resolution * resolution` entries.
    pub fn from_samples(
        samples: Vec<u16>,
        resolution: usize,
        cell_size: f32,
        height_scale: f32,
        min_height: f32,
    ) -> Result<Self, &'static str> {
        if samples.len() != resolution * resolution {
            return Err("height sample count does not match resolution");
        }

        let max_sample = f32::from(samples.iter().copied().max().unwrap_or(0));
        Ok(Self {
            resolution,
            cell_size,
            height_scale,
            min_height,
            max_height: min_height + max_sample * height_scale,
            samples,
        })
    }

    #[must_use]
    pub fn height_at(&self, x: usize, y: usize) -> Option<f32> {
        if x >= self.resolution || y >= self.resolution {
            return None;
        }
        let sample = f32::from(self.samples[y * self.resolution + x]);
        Some(sample.mul_add(self.height_scale, self.min_height))
    }

    #[must_use]
    pub fn bilinear_height(&self, x: f32, y: f32) -> Option<f32> {
        if self.resolution == 0 {
            return None;
        }

        let last = self.resolution - 1;
        let max_coord = last.exact_f32();
        if !(0.0..=max_coord).contains(&x) || !(0.0..=max_coord).contains(&y) {
            return None;
        }

        let x0 = clamped_index(x, last);
        let y0 = clamped_index(y, last);
        let x1 = (x0 + 1).min(last);
        let y1 = (y0 + 1).min(last);
        let fx = x - x0.exact_f32();
        let fy = y - y0.exact_f32();

        let h00 = self.height_at(x0, y0)?;
        let h10 = self.height_at(x1, y0)?;
        let h01 = self.height_at(x0, y1)?;
        let h11 = self.height_at(x1, y1)?;

        let h0 = h00.lerp(h10, fx);
        let h1 = h01.lerp(h11, fy);
        Some(h0.lerp(h1, fy))
    }
}
