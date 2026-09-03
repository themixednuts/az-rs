use glam::Vec2;
use serde::{Deserialize, Serialize};

/// Integer terrain tile coordinate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainCoord {
    pub x: i32,

    pub y: i32,
}

/// Two-dimensional terrain footprint in world units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TerrainBounds {
    pub min: Vec2,

    pub max: Vec2,
}

/// Vertical terrain range in world units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TerrainHeightRange {
    pub min: f32,

    pub max: f32,
}

/// Query and processing spacing for terrain data in world units.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TerrainResolution {
    pub height_spacing: f32,

    pub surface_spacing: f32,
}
