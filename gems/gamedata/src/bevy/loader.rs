//! Bevy loader for native `GameData` table assets at catalog paths.

use std::sync::Arc;

use bevy::asset::io::Reader;
use bevy::asset::{Asset, AssetLoader, AsyncReadExt, LoadContext};
use bevy::reflect::TypePath;
use gamedata::{GameDataError, TableAsset};

/// Loaded native table asset exposed to Bevy systems.
#[derive(Asset, Debug, TypePath)]
pub struct GameDataAsset {
    pub table: Arc<TableAsset>,
}

/// Error returned when a catalog path does not contain a native `GameData` table asset.
#[derive(Debug, thiserror::Error)]
pub enum GameDataLoadError {
    #[error("read GameData table asset: {0}")]
    Io(#[from] std::io::Error),

    #[error("{0}")]
    Decode(#[from] GameDataError),
}

/// Loads catalog paths that contain native `GameData` table assets.
///
/// Legacy/project source bytes are rejected; import tooling must transform them first.
#[derive(Default, TypePath)]
pub struct GameDataAssetLoader;

impl AssetLoader for GameDataAssetLoader {
    type Asset = GameDataAsset;
    type Settings = ();
    type Error = GameDataLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        AsyncReadExt::read_to_end(reader, &mut bytes).await?;
        let table = Arc::new(TableAsset::from_bytes(bytes)?);
        Ok(GameDataAsset { table })
    }

    fn extensions(&self) -> &[&str] {
        &["aztbl"]
    }
}
