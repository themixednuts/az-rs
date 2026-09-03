use az_core::AssetPathBuf;
use glam::Vec2;
use serde::{Deserialize, Serialize};

use crate::TerrainImageChannel;

/// Editable terrain heightmap source.
///
/// Samples are stored in terrain-space row-major order. `(0, 0)` is the
/// bottom-left sample of the height tile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainHeightmapSource {
    pub name: String,

    pub width: u32,

    pub height: u32,

    pub samples: Vec<u16>,
}

/// Required height provider for a terrain region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TerrainHeightSource {
    Image(TerrainHeightImageSource),
    Tiled(TerrainHeightTilesSource),
    Graph(TerrainHeightGraphSource),
    Constant(TerrainConstantHeightSource),
}

/// Height sampled from a texture channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainHeightImageSource {
    pub image: AssetPathBuf,

    pub channel: TerrainImageChannel,

    pub mip: u32,

    pub tiling: Vec2,
}

/// Height sampled from a tiled terrain-height asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainHeightTilesSource {
    pub asset: AssetPathBuf,
}

/// Height produced by a terrain graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerrainHeightGraphSource {
    pub graph: AssetPathBuf,
}

/// Constant terrain elevation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TerrainConstantHeightSource {
    pub value: f32,
}
