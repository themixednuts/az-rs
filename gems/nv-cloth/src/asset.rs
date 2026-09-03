use az_nv_cloth::{
    CLOTH_FABRIC_PRODUCT_EXTENSION, CLOTH_MATERIAL_PRODUCT_EXTENSION, ClothCodecError,
    ClothFabricAsset, ClothMaterialAsset, read_cloth_fabric, read_cloth_material,
};
use bevy::asset::{AssetLoader, AsyncReadExt, LoadContext, io::Reader};
use bevy::reflect::TypePath;

#[derive(Default, TypePath)]
pub struct ClothFabricAssetLoader;

#[derive(Default, TypePath)]
pub struct ClothMaterialAssetLoader;

#[derive(Debug, thiserror::Error)]
pub enum ClothAssetLoadError {
    #[error("read cloth product: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Codec(#[from] ClothCodecError),
}

impl AssetLoader for ClothFabricAssetLoader {
    type Asset = ClothFabricAsset;
    type Settings = ();
    type Error = ClothAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(read_cloth_fabric(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[CLOTH_FABRIC_PRODUCT_EXTENSION]
    }
}

impl AssetLoader for ClothMaterialAssetLoader {
    type Asset = ClothMaterialAsset;
    type Settings = ();
    type Error = ClothAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(read_cloth_material(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[CLOTH_MATERIAL_PRODUCT_EXTENSION]
    }
}
