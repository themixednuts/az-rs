//! Heightmap storage and constructors.

use bevy::prelude::*;

/// Terrain height storage.
pub type TerrainHeight = f32;

/// Heightmap component
///
/// Stores height data for a terrain chunk using a 2D grid.
/// Heights are stored in row-major order (z-major, x-minor).
///
/// # Examples
///
/// ```rust
/// use az_gem_legacy_terrain::Heightmap;
///
/// // Create a 129x129 flat heightmap at height 10.0
/// let heightmap = Heightmap::flat(129, 10.0);
///
/// // Create from existing height data
/// let heights = vec![0.0, 1.0, 2.0, 3.0]; // 2x2 grid
/// let heightmap = Heightmap::from_heights(heights, 2).expect("Valid height data");
///
/// // Get height at coordinates
/// let height = heightmap.get_height(0, 0);
/// ```
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct Heightmap {
    /// Height data in row-major order [z][x]
    pub(super) heights: Vec<TerrainHeight>,

    /// Resolution (vertices per side)
    pub(super) resolution: usize,

    /// Minimum height in this heightmap
    pub(super) min_height: f32,

    /// Maximum height in this heightmap
    pub(super) max_height: f32,
}

impl Default for Heightmap {
    fn default() -> Self {
        Self::new(129) // 129x129 default
    }
}

impl Heightmap {
    /// Create a heightmap with the given resolution.
    #[must_use]
    pub fn new(resolution: usize) -> Self {
        Self {
            heights: vec![0.0; resolution * resolution],
            resolution,
            min_height: 0.0,
            max_height: 0.0,
        }
    }

    /// Create a flat heightmap at the given height.
    #[must_use]
    pub fn flat(resolution: usize, height: f32) -> Self {
        Self {
            heights: vec![height; resolution * resolution],
            resolution,
            min_height: height,
            max_height: height,
        }
    }

    /// Create heightmap from raw height data
    ///
    /// # Errors
    ///
    /// Returns `"Heights vector size doesn't match resolution"` if `heights`
    /// does not hold exactly `resolution * resolution` entries.
    pub fn from_heights(
        heights: Vec<TerrainHeight>,
        resolution: usize,
    ) -> Result<Self, &'static str> {
        if heights.len() != resolution * resolution {
            return Err("Heights vector size doesn't match resolution");
        }

        let (min_height, max_height) = heights
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(min, max), &height| {
                (min.min(height), max.max(height))
            });

        Ok(Self {
            heights,
            resolution,
            min_height,
            max_height,
        })
    }
}
