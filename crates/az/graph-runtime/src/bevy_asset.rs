//! Bevy asset integration for packed graph products.

use bevy_app::{App, Plugin};
use bevy_asset::io::Reader;
use bevy_asset::{Asset, AssetApp, AssetLoader, LoadContext};
use bevy_reflect::TypePath;
use thiserror::Error;

use crate::{PACKED_GRAPH_IR_PRODUCT_EXTENSION, PackedGraphIrError, PackedGraphProduct};

/// Loaded packed graph product.
#[derive(Asset, Debug, Clone, TypePath)]
pub struct PackedGraphAsset {
    product: PackedGraphProduct,
}

impl PackedGraphAsset {
    #[must_use]
    pub const fn new(product: PackedGraphProduct) -> Self {
        Self { product }
    }

    #[must_use]
    pub const fn product(&self) -> &PackedGraphProduct {
        &self.product
    }
}

#[derive(Default, TypePath)]
pub struct PackedGraphAssetLoader;

#[derive(Debug, Error)]
pub enum PackedGraphLoadError {
    #[error("read packed graph asset: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Decode(#[from] PackedGraphIrError),
}

impl AssetLoader for PackedGraphAssetLoader {
    type Asset = PackedGraphAsset;
    type Settings = ();
    type Error = PackedGraphLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let product = PackedGraphProduct::from_bytes(bytes.into_boxed_slice())?;
        Ok(PackedGraphAsset::new(product))
    }

    fn extensions(&self) -> &[&str] {
        &[PACKED_GRAPH_IR_PRODUCT_EXTENSION]
    }
}

/// Registers the generic packed graph asset type and `.azgir` loader.
pub struct PackedGraphAssetPlugin;

impl Plugin for PackedGraphAssetPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<PackedGraphAsset>()
            .init_asset_loader::<PackedGraphAssetLoader>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_graph_loader_claims_azgir_extension() {
        let loader = PackedGraphAssetLoader;
        assert_eq!(loader.extensions(), &["azgir"]);
    }
}
