//! Region-heightmap conversion helpers.

use super::core::Heightmap;
use super::math::remap_index;

impl Heightmap {
    /// Create a heightmap from a terrain region height field.
    ///
    /// # Errors
    ///
    /// Returns any error [`Self::from_region_heightmap_resampled`] returns.
    pub fn from_region_heightmap(region: &crate::RegionHeightmap) -> Result<Self, &'static str> {
        Self::from_region_heightmap_resampled(region, region.resolution)
    }

    /// Create a heightmap from a terrain region height field at a target
    /// resolution.
    ///
    /// # Errors
    ///
    /// Returns `"region heightmap resolution must be non-zero"` for an empty
    /// `region`, `"target heightmap resolution must be non-zero"` for a
    /// `resolution` of `0`, or any error [`Self::from_heights`] returns.
    pub fn from_region_heightmap_resampled(
        region: &crate::RegionHeightmap,
        resolution: usize,
    ) -> Result<Self, &'static str> {
        if region.resolution == 0 {
            return Err("region heightmap resolution must be non-zero");
        }
        if resolution == 0 {
            return Err("target heightmap resolution must be non-zero");
        }

        let max_source = region.resolution - 1;
        let max_target = resolution - 1;
        let mut heights = Vec::with_capacity(resolution * resolution);
        for z in 0..resolution {
            let source_z = remap_index(z, max_target, max_source);
            for x in 0..resolution {
                let source_x = remap_index(x, max_target, max_source);
                let height = region.height_at(source_x, source_z).unwrap_or(0.0);
                heights.push(height);
            }
        }

        Self::from_heights(heights, resolution)
    }
}
