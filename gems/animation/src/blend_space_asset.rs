use az_animation::blend_space_asset::{
    BlendSpaceAsset, BlendSpaceAssetCodecError, CombinedBlendSpaceAsset, read_blend_space_asset,
    read_combined_blend_space_asset,
};
use bevy::{
    asset::{AssetLoader, AsyncReadExt, LoadContext, io::Reader},
    reflect::TypePath,
};

pub const BLEND_SPACE_PRODUCT_EXTENSION: &str = "blend-space.bin";
pub const COMBINED_BLEND_SPACE_PRODUCT_EXTENSION: &str = "combined-blend-space.bin";

#[derive(Default, TypePath)]
pub struct BlendSpaceAssetLoader;

#[derive(Default, TypePath)]
pub struct CombinedBlendSpaceAssetLoader;

#[derive(Debug, thiserror::Error)]
pub enum BlendSpaceAssetLoadError {
    #[error("read blend-space product: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse blend-space product: {0}")]
    Parse(#[from] BlendSpaceAssetCodecError),
}

impl AssetLoader for BlendSpaceAssetLoader {
    type Asset = BlendSpaceAsset;
    type Settings = ();
    type Error = BlendSpaceAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(read_blend_space_asset(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[BLEND_SPACE_PRODUCT_EXTENSION]
    }
}

impl AssetLoader for CombinedBlendSpaceAssetLoader {
    type Asset = CombinedBlendSpaceAsset;
    type Settings = ();
    type Error = BlendSpaceAssetLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        Ok(read_combined_blend_space_asset(&bytes)?)
    }

    fn extensions(&self) -> &[&str] {
        &[COMBINED_BLEND_SPACE_PRODUCT_EXTENSION]
    }
}
