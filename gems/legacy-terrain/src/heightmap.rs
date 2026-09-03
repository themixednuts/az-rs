//! Heightmap data.

mod accessors;
mod conversion;
mod core;
pub(crate) mod math;
mod sampling;

pub use core::{Heightmap, TerrainHeight};

#[cfg(test)]
mod tests {
    use super::*;

    // Every height here is `sample * 0.5 - 5.0` over samples 0..=30, so the
    // bounds are exact halves of small integers; an epsilon would stop the
    // test from pinning the scaling.
    #[allow(clippy::float_cmp)]
    #[test]
    fn converts_region_heightmap_to_component_heightmap() {
        let region =
            crate::RegionHeightmap::from_samples(vec![0, 10, 20, 30], 2, 2.0, 0.5, -5.0).unwrap();

        let heightmap = Heightmap::from_region_heightmap(&region).unwrap();

        assert_eq!(heightmap.resolution(), 2);
        assert_eq!(heightmap.get_height(0, 0), Some(-5.0));
        assert_eq!(heightmap.get_height(1, 1), Some(10.0));
        assert_eq!(heightmap.min_height(), -5.0);
        assert_eq!(heightmap.max_height(), 10.0);
    }

    #[test]
    fn resamples_region_heightmap_to_target_resolution() {
        let region = crate::RegionHeightmap::from_samples(
            vec![0, 10, 20, 30, 40, 50, 60, 70, 80],
            3,
            1.0,
            1.0,
            0.0,
        )
        .unwrap();

        let heightmap = Heightmap::from_region_heightmap_resampled(&region, 2).unwrap();

        assert_eq!(heightmap.resolution(), 2);
        assert_eq!(heightmap.get_height(0, 0), Some(0.0));
        assert_eq!(heightmap.get_height(1, 0), Some(20.0));
        assert_eq!(heightmap.get_height(0, 1), Some(60.0));
        assert_eq!(heightmap.get_height(1, 1), Some(80.0));
    }
}
