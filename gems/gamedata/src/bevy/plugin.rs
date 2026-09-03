//! Bevy plugin registering native `GameData` assets.

use bevy::prelude::*;

use super::{GameDataAsset, GameDataAssetLoader, tables};

/// Registers native `GameData` Bevy assets and loaders.
pub struct GameDataPlugin;

impl Plugin for GameDataPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<GameDataAsset>()
            .init_asset_loader::<GameDataAssetLoader>()
            .init_resource::<tables::GameDataTables>()
            .add_systems(Startup, tables::index_gamedata_catalog)
            .add_systems(
                Update,
                (
                    tables::queue_requested_products,
                    tables::collect_loaded_products,
                )
                    .chain(),
            );
    }
}
