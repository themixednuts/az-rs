//! Terrain engine Bevy asset loaders.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;

use crate::TerrainRegionAsset;

use super::bevy_region_reader::read_terrain_region_asset_from_bevy_reader;
use super::constants::{TERRAIN_REGION_ASSET_EXTENSIONS, TERRAIN_WORLD_MANIFEST_EXTENSIONS};
use super::error::{TerrainRegionAssetFormatError, TerrainWorldManifestFormatError};
use super::manifest::{TerrainWorldManifest, read_terrain_world_manifest_from_bevy_reader};

/// Bevy asset loader for engine terrain-region assets.
#[derive(Default, TypePath)]
pub struct TerrainRegionAssetLoader;

impl AssetLoader for TerrainRegionAssetLoader {
    type Asset = TerrainRegionAsset;
    type Settings = ();
    type Error = TerrainRegionAssetFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        read_terrain_region_asset_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        TERRAIN_REGION_ASSET_EXTENSIONS
    }
}

/// Bevy asset loader for engine terrain world manifests.
#[derive(Default, TypePath)]
pub struct TerrainWorldManifestLoader;

impl AssetLoader for TerrainWorldManifestLoader {
    type Asset = TerrainWorldManifest;
    type Settings = ();
    type Error = TerrainWorldManifestFormatError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        read_terrain_world_manifest_from_bevy_reader(reader).await
    }

    fn extensions(&self) -> &[&str] {
        TERRAIN_WORLD_MANIFEST_EXTENSIONS
    }
}
