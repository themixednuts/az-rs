use glam::Vec2;

use crate::{TerrainBounds, TerrainHeightRange, TerrainRegionRef, TerrainResolution};

#[cfg_attr(feature = "bevy", derive(bevy_asset::Asset, bevy_reflect::TypePath))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainHeightmapAsset {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub samples: Vec<u16>,
}

/// One loaded heightmap tile and the world-space metadata required to sample it.
#[derive(Debug, Clone, Copy)]
pub struct TerrainHeightmapRegionView<'a> {
    heightmap: &'a TerrainHeightmapAsset,
    height_range: TerrainHeightRange,
    region: &'a TerrainRegionRef,
    resolution: TerrainResolution,
    world_bounds: TerrainBounds,
}

impl<'a> TerrainHeightmapRegionView<'a> {
    #[must_use]
    pub const fn new(
        heightmap: &'a TerrainHeightmapAsset,
        height_range: TerrainHeightRange,
        region: &'a TerrainRegionRef,
        resolution: TerrainResolution,
        world_bounds: TerrainBounds,
    ) -> Self {
        Self {
            heightmap,
            height_range,
            region,
            resolution,
            world_bounds,
        }
    }

    #[must_use]
    pub const fn heightmap(self) -> &'a TerrainHeightmapAsset {
        self.heightmap
    }

    #[must_use]
    pub const fn region(self) -> &'a TerrainRegionRef {
        self.region
    }

    #[must_use]
    pub const fn resolution(self) -> TerrainResolution {
        self.resolution
    }

    #[must_use]
    pub const fn world_bounds(self) -> TerrainBounds {
        self.world_bounds
    }

    fn contains_world_xy(self, world_x: f32, world_y: f32) -> bool {
        world_x >= self.region.bounds.min.x
            && (world_x < self.region.bounds.max.x
                || is_terminal_coordinate(
                    world_x,
                    self.region.bounds.max.x,
                    self.world_bounds.max.x,
                ))
            && world_y >= self.region.bounds.min.y
            && (world_y < self.region.bounds.max.y
                || is_terminal_coordinate(
                    world_y,
                    self.region.bounds.max.y,
                    self.world_bounds.max.y,
                ))
    }

    fn lattice_height(self, world_x: f32, world_y: f32) -> Option<f32> {
        let spacing = self.resolution.height_spacing;
        if !spacing.is_finite() || spacing <= 0.0 {
            return None;
        }
        if world_x < self.region.bounds.min.x - LATTICE_TOLERANCE
            || world_x >= self.region.bounds.max.x
            || world_y < self.region.bounds.min.y - LATTICE_TOLERANCE
            || world_y >= self.region.bounds.max.y
        {
            return None;
        }
        let local_x = (world_x - self.region.bounds.min.x) / spacing;
        let local_y = (world_y - self.region.bounds.min.y) / spacing;
        if !local_x.is_finite() || !local_y.is_finite() {
            return None;
        }
        let sample_x = local_x.round();
        let sample_y = local_y.round();
        if (local_x - sample_x).abs() > LATTICE_TOLERANCE
            || (local_y - sample_y).abs() > LATTICE_TOLERANCE
        {
            return None;
        }
        let encoded = self
            .heightmap
            .sample_terrain_xy(sample_index(sample_x)?, sample_index(sample_y)?)?;
        Some(self.height_range.decode_sample(encoded))
    }

    fn terminal_lattice_height(self, world_x: f32, world_y: f32) -> Option<f32> {
        let spacing = self.resolution.height_spacing;
        if !spacing.is_finite() || spacing <= 0.0 {
            return None;
        }
        let sample_x = terminal_sample_index(
            (world_x - self.region.bounds.min.x) / spacing,
            self.heightmap.width,
            world_x,
            self.region.bounds.max.x,
            self.world_bounds.max.x,
        )?;
        let sample_y = terminal_sample_index(
            (world_y - self.region.bounds.min.y) / spacing,
            self.heightmap.height,
            world_y,
            self.region.bounds.max.y,
            self.world_bounds.max.y,
        )?;
        let encoded = self.heightmap.sample_terrain_xy(sample_x, sample_y)?;
        Some(self.height_range.decode_sample(encoded))
    }
}

const LATTICE_TOLERANCE: f32 = 0.001;

/// The first `f32` value outside `u32` range. `u32::MAX` has no exact `f32`
/// representation — it rounds up to this — so range checks are exclusive
/// against this bound rather than inclusive against `u32::MAX`.
const U32_RANGE_LIMIT: f32 = 4_294_967_296.0;

/// A lattice coordinate converted to a sample index, rejecting anything the
/// conversion could not represent exactly.
///
/// Returns `None` for NaN, infinities, negatives, and values at or past
/// [`U32_RANGE_LIMIT`].
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the guard rejects NaN, infinities, negatives and out-of-range values first, so the cast is exact; Rust has no `TryFrom<f32> for u32`"
)]
fn sample_index(value: f32) -> Option<u32> {
    // Excludes NaN and both infinities as well as out-of-range magnitudes.
    if !(0.0..U32_RANGE_LIMIT).contains(&value) {
        return None;
    }
    Some(value as u32)
}

/// A sample coordinate widened to `f32` for interpolation math.
///
/// Heightmap extents are bounded by the sample buffer that backs them, so a
/// coordinate never approaches 2^24, the point where `f32` stops representing
/// consecutive integers.
#[allow(
    clippy::cast_precision_loss,
    reason = "sample coordinates are bounded far below 2^24 by the backing buffer; there is no lossless `u32` -> `f32` conversion"
)]
fn sample_coordinate(value: u32) -> f32 {
    debug_assert!(
        value < (1 << 24),
        "heightmap sample coordinate exceeds the exact `f32` integer range"
    );
    value as f32
}

fn is_terminal_coordinate(value: f32, region_max: f32, world_max: f32) -> bool {
    (value - region_max).abs() <= LATTICE_TOLERANCE
        && (region_max - world_max).abs() <= LATTICE_TOLERANCE
}

fn terminal_sample_index(
    local_coordinate: f32,
    sample_count: u32,
    world_coordinate: f32,
    region_max: f32,
    world_max: f32,
) -> Option<u32> {
    if !local_coordinate.is_finite() || sample_count == 0 {
        return None;
    }
    let rounded = local_coordinate.round();
    if (local_coordinate - rounded).abs() > LATTICE_TOLERANCE {
        return None;
    }
    let index = sample_index(rounded)?;
    if index < sample_count {
        return Some(index);
    }
    (index == sample_count && is_terminal_coordinate(world_coordinate, region_max, world_max))
        .then_some(sample_count - 1)
}

/// A height sampled from a tiled terrain world together with its owning region.
#[derive(Debug, Clone, Copy)]
pub struct TerrainWorldHeightSample<'a> {
    height: f32,
    region: TerrainHeightmapRegionView<'a>,
}

impl<'a> TerrainWorldHeightSample<'a> {
    #[must_use]
    pub const fn height(self) -> f32 {
        self.height
    }

    #[must_use]
    pub const fn region(self) -> TerrainHeightmapRegionView<'a> {
        self.region
    }
}

/// Bilinearly sample a tiled terrain world without flattening tile boundaries.
///
/// A query inside the final sample cell of one region reads the missing corner
/// samples from the neighboring region. At the authored world's terminal max
/// edge, where no neighboring tile can exist, the final in-bounds sample is
/// extended through the remaining cell. A missing interior neighbor still
/// rejects the query rather than flattening a hole in the terrain world.
#[must_use]
pub fn bilinear_world_height<'a, Regions>(
    regions: Regions,
    world_x: f32,
    world_y: f32,
) -> Option<TerrainWorldHeightSample<'a>>
where
    Regions: Clone + IntoIterator<Item = TerrainHeightmapRegionView<'a>>,
{
    if !world_x.is_finite() || !world_y.is_finite() {
        return None;
    }
    let owner = regions
        .clone()
        .into_iter()
        .filter(|region| region.contains_world_xy(world_x, world_y))
        .max_by_key(|region| region.region.priority)?;
    let spacing = owner.resolution.height_spacing;
    if !spacing.is_finite() || spacing <= 0.0 {
        return None;
    }
    let origin = owner.region.bounds.min;
    let local = (Vec2::new(world_x, world_y) - origin) / spacing;
    let floor = local.floor();
    let fraction = local - floor;
    // A coordinate already sitting on the lattice interpolates against itself
    // instead of reaching into the next cell.
    let next = Vec2::new(
        if fraction.x <= f32::EPSILON {
            floor.x
        } else {
            floor.x + 1.0
        },
        if fraction.y <= f32::EPSILON {
            floor.y
        } else {
            floor.y + 1.0
        },
    );
    let world_low = floor.mul_add(Vec2::splat(spacing), origin);
    let world_high = origin + next * spacing;
    let height_00 = lattice_height(regions.clone(), owner, world_low.x, world_low.y)?;
    let height_10 = lattice_height(regions.clone(), owner, world_high.x, world_low.y)?;
    let height_01 = lattice_height(regions.clone(), owner, world_low.x, world_high.y)?;
    let height_11 = lattice_height(regions, owner, world_high.x, world_high.y)?;
    let row_low = (height_10 - height_00).mul_add(fraction.x, height_00);
    let row_high = (height_11 - height_01).mul_add(fraction.x, height_01);
    Some(TerrainWorldHeightSample {
        height: (row_high - row_low).mul_add(fraction.y, row_low),
        region: owner,
    })
}

fn lattice_height<'a>(
    regions: impl IntoIterator<Item = TerrainHeightmapRegionView<'a>>,
    owner: TerrainHeightmapRegionView<'a>,
    world_x: f32,
    world_y: f32,
) -> Option<f32> {
    regions
        .into_iter()
        .filter_map(|region| {
            region
                .lattice_height(world_x, world_y)
                .map(|height| (region.region.priority, height))
        })
        .max_by_key(|(priority, _)| *priority)
        .map(|(_, height)| height)
        .or_else(|| owner.terminal_lattice_height(world_x, world_y))
}

impl TerrainHeightmapAsset {
    /// Read a height sample in terrain coordinates, where `(0, 0)` is the
    /// bottom-left sample.
    ///
    /// The authored and processed heightmap contracts both store terrain-space
    /// rows. Image-space conversion belongs to the source importer and must not
    /// be repeated at runtime.
    #[must_use]
    pub fn sample_terrain_xy(&self, x: u32, y: u32) -> Option<u16> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = u64::from(y)
            .checked_mul(u64::from(self.width))?
            .checked_add(u64::from(x))?;
        self.samples.get(usize::try_from(index).ok()?).copied()
    }

    /// Bilinearly sample a terrain-space height in sample coordinates.
    #[must_use]
    pub fn bilinear_height(&self, range: crate::TerrainHeightRange, x: f32, y: f32) -> Option<f32> {
        let max_x = sample_coordinate(self.width.checked_sub(1)?);
        let max_y = sample_coordinate(self.height.checked_sub(1)?);
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=max_x).contains(&x)
            || !(0.0..=max_y).contains(&y)
        {
            return None;
        }

        let x0 = sample_index(x.floor())?;
        let y0 = sample_index(y.floor())?;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);
        let fx = x - sample_coordinate(x0);
        let fy = y - sample_coordinate(y0);
        let h00 = range.decode_sample(self.sample_terrain_xy(x0, y0)?);
        let h10 = range.decode_sample(self.sample_terrain_xy(x1, y0)?);
        let h01 = range.decode_sample(self.sample_terrain_xy(x0, y1)?);
        let h11 = range.decode_sample(self.sample_terrain_xy(x1, y1)?);
        let h0 = (h10 - h00).mul_add(fx, h00);
        let h1 = (h11 - h01).mul_add(fx, h01);
        Some((h1 - h0).mul_add(fy, h0))
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec2;

    use super::*;

    /// Heights come out of a decode-then-interpolate chain, so compare against a
    /// tolerance rather than pinning an exact bit pattern.
    fn assert_height(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1e-4,
            "expected height {expected}, got {actual}"
        );
    }

    #[test]
    fn samples_processed_terrain_rows_and_decodes_world_height() {
        let asset = TerrainHeightmapAsset {
            name: "r_+00_+00".to_string(),
            width: 2,
            height: 2,
            samples: vec![0, u16::MAX, u16::MAX / 2, u16::MAX / 2],
        };
        let range = crate::TerrainHeightRange {
            min: -10.0,
            max: 90.0,
        };

        assert_eq!(asset.sample_terrain_xy(0, 0), Some(0));
        assert_eq!(asset.sample_terrain_xy(1, 0), Some(u16::MAX));
        assert_eq!(asset.sample_terrain_xy(0, 1), Some(u16::MAX / 2));
        assert_eq!(asset.bilinear_height(range, 0.5, 0.5), Some(39.99962));
        assert_eq!(asset.bilinear_height(range, f32::NAN, 0.0), None);
        assert_eq!(asset.bilinear_height(range, 2.0, 0.0), None);
    }

    #[test]
    fn samples_across_authored_region_seams_without_clamping() {
        let left_heightmap = TerrainHeightmapAsset {
            name: "left".to_owned(),
            width: 2,
            height: 2,
            samples: vec![0; 4],
        };
        let right_heightmap = TerrainHeightmapAsset {
            name: "right".to_owned(),
            width: 2,
            height: 2,
            samples: vec![u16::MAX; 4],
        };
        let left_region = region_ref("left", 0.0, 2.0);
        let right_region = region_ref("right", 2.0, 4.0);
        let range = TerrainHeightRange {
            min: 0.0,
            max: 100.0,
        };
        let resolution = TerrainResolution {
            height_spacing: 1.0,
            surface_spacing: 1.0,
        };
        let regions = [
            TerrainHeightmapRegionView::new(
                &left_heightmap,
                range,
                &left_region,
                resolution,
                world_bounds(4.0),
            ),
            TerrainHeightmapRegionView::new(
                &right_heightmap,
                range,
                &right_region,
                resolution,
                world_bounds(4.0),
            ),
        ];

        let seam = bilinear_world_height(regions, 1.5, 0.5).expect("neighbor supplies seam");
        assert_height(seam.height(), 50.0);
        assert_eq!(seam.region().region().asset, "left");
        assert_height(
            bilinear_world_height(regions, 2.0, 0.5).unwrap().height(),
            100.0,
        );
    }

    #[test]
    fn extends_the_final_world_edge_from_the_last_authored_sample() {
        let heightmap = TerrainHeightmapAsset {
            name: "left".to_owned(),
            width: 2,
            height: 2,
            samples: vec![0; 4],
        };
        let region = region_ref("left", 0.0, 2.0);
        let view = TerrainHeightmapRegionView::new(
            &heightmap,
            TerrainHeightRange {
                min: 0.0,
                max: 100.0,
            },
            &region,
            TerrainResolution {
                height_spacing: 1.0,
                surface_spacing: 1.0,
            },
            world_bounds(2.0),
        );

        assert_height(
            bilinear_world_height([view], 1.5, 0.5).unwrap().height(),
            0.0,
        );
        assert_height(
            bilinear_world_height([view], 2.0, 0.5).unwrap().height(),
            0.0,
        );
    }

    #[test]
    fn rejects_a_missing_interior_neighbor_before_a_farther_region() {
        let left_heightmap = TerrainHeightmapAsset {
            name: "left".to_owned(),
            width: 2,
            height: 2,
            samples: vec![0; 4],
        };
        let far_heightmap = TerrainHeightmapAsset {
            name: "far".to_owned(),
            width: 2,
            height: 2,
            samples: vec![u16::MAX; 4],
        };
        let left_region = region_ref("left", 0.0, 2.0);
        let far_region = region_ref("far", 4.0, 6.0);
        let range = TerrainHeightRange {
            min: 0.0,
            max: 100.0,
        };
        let resolution = TerrainResolution {
            height_spacing: 1.0,
            surface_spacing: 1.0,
        };
        let regions = [
            TerrainHeightmapRegionView::new(
                &left_heightmap,
                range,
                &left_region,
                resolution,
                world_bounds(6.0),
            ),
            TerrainHeightmapRegionView::new(
                &far_heightmap,
                range,
                &far_region,
                resolution,
                world_bounds(6.0),
            ),
        ];

        assert!(bilinear_world_height(regions, 1.5, 0.5).is_none());
    }

    fn region_ref(name: &str, min_x: f32, max_x: f32) -> TerrainRegionRef {
        TerrainRegionRef {
            asset: name.to_owned(),
            coord: None,
            bounds: crate::TerrainBounds {
                min: Vec2::new(min_x, 0.0),
                max: Vec2::new(max_x, 2.0),
            },
            priority: 0,
        }
    }

    fn world_bounds(max_x: f32) -> TerrainBounds {
        TerrainBounds {
            min: Vec2::ZERO,
            max: Vec2::new(max_x, 2.0),
        }
    }
}
