//! Heightmap mutation and accessors.

use super::core::{Heightmap, TerrainHeight};
use super::math::ExactF32;

impl Heightmap {
    /// Apply procedural noise to heightmap
    pub fn apply_noise(&mut self, scale: f32, amplitude: f32, octaves: u32, seed: u64) {
        // The seed only offsets the hash phase, and `sin` of a value with the
        // magnitude of a full `u64` carries no information anyway, so fold the
        // seed down to a range `f32` holds exactly.
        let seed_phase = f32::from(u16::try_from(seed & 0xFFFF).unwrap_or(0));

        for z in 0..self.resolution {
            for x in 0..self.resolution {
                let mut height = 0.0;
                let mut freq = 1.0;
                let mut amp = 1.0;

                // Multi-octave Perlin-like noise (simplified)
                for _ in 0..octaves {
                    let nx = x.exact_f32() * scale * freq / self.resolution.exact_f32();
                    let nz = z.exact_f32() * scale * freq / self.resolution.exact_f32();

                    // Simple value noise (replace with proper Perlin later)
                    let noise_val =
                        ((nx * 12.9898 + nz * 78.233 + seed_phase).sin() * 43_758.547).fract();
                    height = ((noise_val * 2.0 - 1.0) * amp).mul_add(amplitude, height);

                    freq *= 2.0;
                    amp *= 0.5;
                }

                let current = self.get_height(x, z).unwrap_or(0.0);
                self.set_height(x, z, current + height);
            }
        }

        self.recalculate_bounds();
    }

    /// Recalculate min/max bounds
    pub fn recalculate_bounds(&mut self) {
        self.min_height = self.heights.iter().copied().fold(f32::INFINITY, f32::min);
        self.max_height = self
            .heights
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
    }

    /// Get resolution
    #[must_use]
    pub const fn resolution(&self) -> usize {
        self.resolution
    }

    /// Get min height
    #[must_use]
    pub const fn min_height(&self) -> f32 {
        self.min_height
    }

    /// Get max height
    #[must_use]
    pub const fn max_height(&self) -> f32 {
        self.max_height
    }

    /// Get raw height data
    #[must_use]
    pub fn heights(&self) -> &[TerrainHeight] {
        &self.heights
    }
}
