//! Terrain engine asset constants.

pub(super) const MAGIC: &[u8; 8] = b"AZTRGN\0\0";
pub(super) const VERSION: u32 = 2;

/// File extensions handled by the terrain-region asset loader.
pub const TERRAIN_REGION_ASSET_EXTENSIONS: &[&str] = &["terrain-region.bin"];
/// File extensions handled by the terrain world manifest loader.
pub const TERRAIN_WORLD_MANIFEST_EXTENSIONS: &[&str] = &["terrain-world.bin"];
