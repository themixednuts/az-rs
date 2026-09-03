use az_core::AssetPathBuf;
use serde::{Deserialize, Serialize};

use crate::TerrainHeightSource;

/// Editable terrain region source.
///
/// Regions hold reusable terrain content. World documents decide where a
/// region is placed, how large its footprint is, and how it blends with other
/// regions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerrainRegionSource {
    pub name: String,

    pub height: TerrainHeightSource,

    pub surface: Option<crate::TerrainSurfaceSource>,

    pub water: Option<AssetPathBuf>,

    pub layers: Option<AssetPathBuf>,
}
