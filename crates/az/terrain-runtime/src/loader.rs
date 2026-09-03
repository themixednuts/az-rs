//! Bevy loaders for native terrain products.

use bevy_asset::io::Reader;
use bevy_asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy_reflect::TypePath;

use crate::{
    TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION, TERRAIN_LAYER_SET_PRODUCT_EXTENSION,
    TERRAIN_REGION_PRODUCT_EXTENSION, TERRAIN_WORLD_PRODUCT_EXTENSION, TerrainAssetCodecError,
    TerrainHeightmapAsset, TerrainLayerSetAsset, TerrainRegionAsset, TerrainWorldAsset,
    decode_terrain_heightmap_asset, decode_terrain_layer_set_asset, decode_terrain_region_asset,
    decode_terrain_world_asset,
};

#[derive(Debug, thiserror::Error)]
pub enum TerrainAssetLoadError {
    #[error("read cooked terrain product: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] TerrainAssetCodecError),
}

macro_rules! terrain_loader {
    ($loader:ident, $asset:ty, $decode:ident, $extension:ident) => {
        #[derive(Default, TypePath)]
        pub struct $loader;

        impl AssetLoader for $loader {
            type Asset = $asset;
            type Settings = ();
            type Error = TerrainAssetLoadError;

            async fn load(
                &self,
                reader: &mut dyn Reader,
                _settings: &Self::Settings,
                _load_context: &mut LoadContext<'_>,
            ) -> Result<Self::Asset, Self::Error> {
                let mut bytes = Vec::new();
                AsyncReadExt::read_to_end(reader, &mut bytes).await?;
                Ok($decode(&bytes)?)
            }

            fn extensions(&self) -> &[&str] {
                &[$extension]
            }
        }
    };
}

terrain_loader!(
    TerrainWorldAssetLoader,
    TerrainWorldAsset,
    decode_terrain_world_asset,
    TERRAIN_WORLD_PRODUCT_EXTENSION
);
terrain_loader!(
    TerrainRegionAssetLoader,
    TerrainRegionAsset,
    decode_terrain_region_asset,
    TERRAIN_REGION_PRODUCT_EXTENSION
);
terrain_loader!(
    TerrainLayerSetAssetLoader,
    TerrainLayerSetAsset,
    decode_terrain_layer_set_asset,
    TERRAIN_LAYER_SET_PRODUCT_EXTENSION
);
terrain_loader!(
    TerrainHeightmapAssetLoader,
    TerrainHeightmapAsset,
    decode_terrain_heightmap_asset,
    TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loaders_claim_distinct_native_product_suffixes() {
        assert_eq!(
            AssetLoader::extensions(&TerrainWorldAssetLoader),
            &[TERRAIN_WORLD_PRODUCT_EXTENSION]
        );
        assert_eq!(
            AssetLoader::extensions(&TerrainRegionAssetLoader),
            &[TERRAIN_REGION_PRODUCT_EXTENSION]
        );
        assert_eq!(
            AssetLoader::extensions(&TerrainLayerSetAssetLoader),
            &[TERRAIN_LAYER_SET_PRODUCT_EXTENSION]
        );
        assert_eq!(
            AssetLoader::extensions(&TerrainHeightmapAssetLoader),
            &[TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION]
        );
    }
}
