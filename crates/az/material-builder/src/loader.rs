//! Decode-only Bevy asset loaders for compiled material products.
//!
//! Loaders dispatch by catalog product-path extension exactly like the
//! `GameData` table loader: `azmaterialtype` for [`MaterialTypeAsset`] and
//! `azmaterial` for [`MaterialAsset`]. Renderer consumption is out of scope
//! for v1 (adopted decision 3); these loaders expose the decoded property
//! tables to runtime systems.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext};
use bevy::reflect::TypePath;

use crate::codec::{MaterialAssetCodecError, decode_material_asset, decode_material_type_asset};
use crate::{
    MATERIAL_PRODUCT_EXTENSION, MATERIAL_TYPE_PRODUCT_EXTENSION, MaterialAsset, MaterialTypeAsset,
};

/// Error returned when a catalog path does not contain a compiled material product.
#[derive(Debug, thiserror::Error)]
pub enum MaterialAssetLoadError {
    #[error("read compiled material product: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Decode(#[from] MaterialAssetCodecError),
}

/// Loads catalog paths that contain compiled `azoth.material.type` products.
#[derive(Default, TypePath)]
pub struct MaterialTypeAssetLoader;

impl AssetLoader for MaterialTypeAssetLoader {
    type Asset = MaterialTypeAsset;
    type Settings = ();
    type Error = MaterialAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(decode_material_type_asset(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[MATERIAL_TYPE_PRODUCT_EXTENSION]
    }
}

/// Loads catalog paths that contain compiled `azoth.material.material` products.
#[derive(Default, TypePath)]
pub struct MaterialAssetLoader;

impl AssetLoader for MaterialAssetLoader {
    type Asset = MaterialAsset;
    type Settings = ();
    type Error = MaterialAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        let asset = decode_material_asset(&bytes)?;
        let _: bevy::asset::Handle<MaterialTypeAsset> =
            load_context.load(asset.material_type.clone());
        if let Some(parent) = asset.parent.as_ref() {
            let _: bevy::asset::Handle<MaterialAsset> = load_context.load(parent.clone());
        }
        Ok(asset)
    }

    fn extensions(&self) -> &[&str] {
        &[MATERIAL_PRODUCT_EXTENSION]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loader_extensions_match_product_extensions() {
        assert_eq!(
            AssetLoader::extensions(&MaterialTypeAssetLoader),
            &[MATERIAL_TYPE_PRODUCT_EXTENSION]
        );
        assert_eq!(
            AssetLoader::extensions(&MaterialAssetLoader),
            &[MATERIAL_PRODUCT_EXTENSION]
        );
    }
}
