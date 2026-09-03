//! Bevy loader for processed `.azmesh` products.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::reflect::TypePath;

use crate::codec::{MeshCodecError, decode_mesh_asset};
use crate::{MESH_PRODUCT_EXTENSION, MeshAsset};

#[derive(Debug, thiserror::Error)]
pub enum MeshAssetLoadError {
    #[error("read processed mesh product: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Decode(#[from] MeshCodecError),
}

#[derive(Default, TypePath)]
pub struct MeshAssetLoader;

impl AssetLoader for MeshAssetLoader {
    type Asset = MeshAsset;
    type Settings = ();
    type Error = MeshAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(decode_mesh_asset(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[MESH_PRODUCT_EXTENSION]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_claims_only_processed_mesh_products() {
        assert_eq!(
            AssetLoader::extensions(&MeshAssetLoader),
            &[MESH_PRODUCT_EXTENSION]
        );
    }
}
