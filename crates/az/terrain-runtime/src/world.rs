use crate::{TerrainBounds, TerrainCoord, TerrainHeightRange, TerrainResolution};

#[cfg_attr(feature = "bevy", derive(bevy_asset::Asset, bevy_reflect::TypePath))]
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainWorldAsset {
    pub name: String,
    pub bounds: TerrainBounds,
    pub height_range: TerrainHeightRange,
    pub resolution: TerrainResolution,
    pub layers: String,
    pub regions: Vec<TerrainRegionRef>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TerrainRegionRef {
    pub asset: String,
    pub coord: Option<TerrainCoord>,
    pub bounds: TerrainBounds,
    pub priority: i32,
}
