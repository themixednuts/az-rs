use bevy::prelude::*;

pub const MIN_TERRAIN_MESH_RESOLUTION: usize = 2;
pub const DEFAULT_TERRAIN_MESH_MAX_RESOLUTION: usize = 257;

/// Mesh resolution policy for terrain region render chunks.
///
/// Lumberyard reference: `dev/Gems/LegacyTerrain/Code/Source/terrain_sector.cpp:21` and
/// `dev/Gems/LegacyTerrain/Code/Source/terrain_sector_render.cpp:574`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum TerrainMeshResolution {
    /// Use the transformed region heightmap resolution.
    Native,
    /// Use the transformed region heightmap resolution up to this cap.
    Max(usize),
    /// Resample every region to this fixed resolution.
    Fixed(usize),
}

impl Default for TerrainMeshResolution {
    fn default() -> Self {
        Self::Max(DEFAULT_TERRAIN_MESH_MAX_RESOLUTION)
    }
}

impl TerrainMeshResolution {
    #[must_use]
    pub const fn resolve(self, native_resolution: usize) -> usize {
        let native_resolution = clamp_terrain_mesh_resolution(native_resolution);
        match self {
            Self::Native => native_resolution,
            Self::Max(max_resolution) => {
                let max_resolution = clamp_terrain_mesh_resolution(max_resolution);
                if native_resolution < max_resolution {
                    native_resolution
                } else {
                    max_resolution
                }
            }
            Self::Fixed(resolution) => clamp_terrain_mesh_resolution(resolution),
        }
    }
}

pub const fn clamp_terrain_mesh_resolution(resolution: usize) -> usize {
    if resolution < MIN_TERRAIN_MESH_RESOLUTION {
        MIN_TERRAIN_MESH_RESOLUTION
    } else {
        resolution
    }
}

/// Render settings for terrain regions.
#[derive(Debug, Clone, Resource, Reflect)]
pub struct TerrainRegionRenderConfig {
    pub mesh_resolution: TerrainMeshResolution,
    pub base_color: Color,
}

impl Default for TerrainRegionRenderConfig {
    fn default() -> Self {
        Self {
            mesh_resolution: TerrainMeshResolution::default(),
            base_color: Color::srgb(0.18, 0.34, 0.24),
        }
    }
}

/// Render settings for terrain water surfaces.
#[derive(Debug, Clone, Resource, Reflect)]
pub struct TerrainWaterRenderConfig {
    pub base_color: Color,
    pub surface_offset: f32,
}

impl Default for TerrainWaterRenderConfig {
    fn default() -> Self {
        Self {
            base_color: Color::srgba(0.06, 0.28, 0.38, 0.62),
            surface_offset: 0.03,
        }
    }
}
