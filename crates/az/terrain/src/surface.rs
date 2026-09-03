use az_core::AssetPathBuf;
use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::SurfaceTag;

/// Optional surface provider for a terrain region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerrainSurfaceSource {
    Image(TerrainSurfaceImageSource),
    Weights(TerrainSurfaceWeightsSource),
    Graph(TerrainSurfaceGraphSource),
}

/// Scalar channel used when sampling an image as terrain data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerrainImageChannel {
    Red,
    Green,
    Blue,
    Alpha,
    Luminance,
}

/// Surface weights sampled from image channels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainSurfaceImageSource {
    pub image: AssetPathBuf,

    pub mip: u32,

    pub tiling: Vec2,

    pub channels: Vec<TerrainSurfaceChannel>,
}

/// Mapping from one image channel to one terrain surface tag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainSurfaceChannel {
    pub channel: TerrainImageChannel,

    pub tag: SurfaceTag,
}

/// Surface weights sampled from a tiled terrain-surface asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainSurfaceWeightsSource {
    pub asset: AssetPathBuf,
}

/// Surface weights produced by a terrain graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainSurfaceGraphSource {
    pub graph: AssetPathBuf,
}
