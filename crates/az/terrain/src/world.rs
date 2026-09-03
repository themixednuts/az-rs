use az_core::AssetPathBuf;
use serde::{Deserialize, Serialize};

use crate::{TerrainBounds, TerrainCoord, TerrainHeightRange, TerrainResolution};

/// Editable terrain world source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainWorldSource {
    pub name: String,

    pub bounds: TerrainBounds,

    pub height_range: TerrainHeightRange,

    pub resolution: TerrainResolution,

    pub layers: AssetPathBuf,

    pub regions: Vec<TerrainRegionRef>,
}

/// Placement of one reusable terrain region in a world.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainRegionRef {
    pub asset: AssetPathBuf,

    pub coord: Option<TerrainCoord>,

    pub bounds: TerrainBounds,

    pub priority: i32,
}
