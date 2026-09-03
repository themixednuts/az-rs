//! Heightmap sampling and normal calculation.

use bevy::prelude::*;

use super::core::Heightmap;
use super::math::{ExactF32, floor_index};

impl Heightmap {
    /// Get height at (x, z) coordinates
    ///
    /// Returns `None` if coordinates are out of bounds.
    #[inline]
    #[must_use]
    pub fn get_height(&self, x: usize, z: usize) -> Option<f32> {
        if x >= self.resolution || z >= self.resolution {
            return None;
        }
        Some(self.heights[z * self.resolution + x])
    }

    /// Get height at (x, z) coordinates with bounds checking
    ///
    /// This is the unsafe version that assumes coordinates are valid.
    /// Use `get_height` for safe access.
    ///
    /// # Safety
    ///
    /// The caller must ensure `x < self.resolution` and `z < self.resolution`.
    #[inline]
    #[must_use]
    pub unsafe fn get_height_unchecked(&self, x: usize, z: usize) -> f32 {
        unsafe { *self.heights.get_unchecked(z * self.resolution + x) }
    }

    /// Set height at (x, z) coordinates
    pub fn set_height(&mut self, x: usize, z: usize, height: f32) {
        if x >= self.resolution || z >= self.resolution {
            return;
        }

        self.heights[z * self.resolution + x] = height;

        // Update min/max
        self.min_height = self.min_height.min(height);
        self.max_height = self.max_height.max(height);
    }

    /// Get interpolated height at fractional coordinates
    ///
    /// Returns `None` if any of the required coordinates are out of bounds.
    #[must_use]
    pub fn get_height_interpolated(&self, x: f32, z: f32) -> Option<f32> {
        let x0 = floor_index(x);
        let z0 = floor_index(z);
        let x1 = (x0 + 1).min(self.resolution - 1);
        let z1 = (z0 + 1).min(self.resolution - 1);

        let fx = x - x0.exact_f32();
        let fz = z - z0.exact_f32();

        // Bilinear interpolation
        let h00 = self.get_height(x0, z0)?;
        let h10 = self.get_height(x1, z0)?;
        let h01 = self.get_height(x0, z1)?;
        let h11 = self.get_height(x1, z1)?;

        let h0 = h00.mul_add(1.0 - fx, h10 * fx);
        let h1 = h01 * (1.0 - fx) + h11 * fx;

        Some(h0 * (1.0 - fz) + h1 * fz)
    }

    /// Calculate normal at (x, z) using finite differences
    ///
    /// Returns `None` if coordinates are out of bounds.
    #[must_use]
    pub fn calculate_normal(&self, x: usize, z: usize, spacing: f32) -> Option<Vec3> {
        let h_center = self.get_height(x, z)?;

        let h_left = if x > 0 {
            self.get_height(x - 1, z).unwrap_or(h_center)
        } else {
            h_center
        };

        let h_right = if x < self.resolution - 1 {
            self.get_height(x + 1, z).unwrap_or(h_center)
        } else {
            h_center
        };

        let h_down = if z > 0 {
            self.get_height(x, z - 1).unwrap_or(h_center)
        } else {
            h_center
        };

        let h_up = if z < self.resolution - 1 {
            self.get_height(x, z + 1).unwrap_or(h_center)
        } else {
            h_center
        };

        // Calculate tangent vectors
        let dx = Vec3::new(2.0 * spacing, h_right - h_left, 0.0);
        let dz = Vec3::new(0.0, h_up - h_down, 2.0 * spacing);

        // Normal is cross product of tangents
        Some(dz.cross(dx).normalize_or_zero())
    }
}
