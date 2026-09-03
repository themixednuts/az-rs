use std::collections::HashSet;

use bevy::prelude::*;

use crate::engine_asset;
use crate::world::TerrainRegionId;

#[derive(Debug, Default, Resource)]
pub(super) struct RenderedTerrainRegions {
    pub(super) regions: HashSet<TerrainRegionId>,
}

#[derive(Debug, Default, Resource)]
pub(super) struct LoadedTerrainWorldManifests {
    pub(super) manifests: HashSet<bevy::asset::AssetId<engine_asset::TerrainWorldManifest>>,
}
