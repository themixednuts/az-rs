use glam::Vec2;

use crate::SurfaceTag;

#[cfg_attr(feature = "bevy", derive(bevy_asset::Asset, bevy_reflect::TypePath))]
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainRegionAsset {
    pub name: String,
    pub height: TerrainHeightSource,
    pub surface: Option<TerrainSurfaceSource>,
    pub water: Option<String>,
    pub layers: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerrainHeightSource {
    Image(TerrainHeightImageSource),
    Tiled(TerrainHeightTilesSource),
    Graph(TerrainHeightGraphSource),
    Constant(TerrainConstantHeightSource),
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainHeightImageSource {
    pub image: String,
    pub channel: TerrainImageChannel,
    pub mip: u32,
    pub tiling: Vec2,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainHeightTilesSource {
    pub asset: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainHeightGraphSource {
    pub graph: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerrainConstantHeightSource {
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerrainSurfaceSource {
    Image(TerrainSurfaceImageSource),
    Weights(TerrainSurfaceWeightsSource),
    Graph(TerrainSurfaceGraphSource),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerrainImageChannel {
    Red,
    Green,
    Blue,
    Alpha,
    Luminance,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainSurfaceImageSource {
    pub image: String,
    pub mip: u32,
    pub tiling: Vec2,
    pub channels: Vec<TerrainSurfaceChannel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainSurfaceChannel {
    pub channel: TerrainImageChannel,
    pub tag: SurfaceTag,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainSurfaceWeightsSource {
    pub asset: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerrainSurfaceGraphSource {
    pub graph: String,
}
