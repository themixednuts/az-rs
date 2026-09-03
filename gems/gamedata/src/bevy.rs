//! Bevy integration for native `GameData` table assets.
//!
//! Discovers native `GameData` table products by asset type and loads them from
//! asset-catalog paths. Projects provide generated merged row schemas and
//! semantic managers; this module owns the engine-side asset, loader, catalog
//! index, request-driven cache, and release wiring.

mod loader;
mod plugin;
mod runtime;
mod tables;

pub use gamedata::TableAsset;
pub use loader::{GameDataAsset, GameDataAssetLoader, GameDataLoadError};
pub use plugin::GameDataPlugin;
pub use runtime::{GameData, GameDataCache};
pub use tables::GameDataTables;

#[cfg(test)]
pub(crate) mod fixtures {
    use az_asset::{AssetCatalog, AssetCatalogEntry, AssetId, write_asset_catalog};

    pub fn encode_asset_catalog(
        entries: &[(AssetId, &str, u32)],
    ) -> Result<Vec<u8>, gamedata::GameDataError> {
        let catalog = AssetCatalog::new(
            entries
                .iter()
                .map(|(asset_id, rel_path, size_bytes)| {
                    AssetCatalogEntry::new(
                        *asset_id,
                        *gamedata::GAMEDATA_TABLE_ASSET_TYPE.as_uuid(),
                        gamedata::GAMEDATA_TABLE_FORMAT_ID.as_str(),
                        gamedata::GAMEDATA_TABLE_VERSION,
                        *rel_path,
                        Some(gamedata::GAMEDATA_TABLE_VERSION),
                        u64::from(*size_bytes),
                        [0; az_asset::PACKAGE_CONTENT_HASH_BYTES],
                    )
                })
                .collect(),
        )
        .map_err(|err| gamedata::GameDataError::Decode(format!("build asset catalog: {err}")))?;
        let mut bytes = Vec::new();
        write_asset_catalog(&catalog, &mut bytes).map_err(|err| {
            gamedata::GameDataError::Decode(format!("encode asset catalog: {err}"))
        })?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use bevy::asset::{AssetPlugin, LoadState};
    use bevy::prelude::*;
    use gamedata::{ASSET_CATALOG_PATH, Row, SchemaHash};
    use uuid::Uuid;

    use super::fixtures::encode_asset_catalog;
    use super::{GameDataAsset, GameDataPlugin};

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
    struct FixtureRow;

    impl Row for FixtureRow {
        const NAME: &'static str = "FixtureRow";
        const CRC: u32 = 0x5060_7080;
    }

    const FIXTURE_TABLE_NAME: &str = "FixtureTable";

    fn write_fixture(root: &Path) {
        let path = "sharedassets/test/fixturetable.aztbl";
        let table_file = root.join(path);
        let table_dir = table_file.parent().expect("table parent");
        fs::create_dir_all(table_dir).expect("create table dir");
        fs::write(
            table_file,
            crate::table::fixture_table_asset_bytes(
                SchemaHash(1),
                FIXTURE_TABLE_NAME,
                FixtureRow::CRC,
                1,
            ),
        )
        .expect("write table");

        let asset_id = az_asset::AssetId::new(Uuid::new_v4(), 0);
        let catalog_bytes = encode_asset_catalog(&[(asset_id, path, 128)]).expect("catalog");
        fs::write(root.join(ASSET_CATALOG_PATH), catalog_bytes).expect("write catalog");
    }

    fn await_asset(
        app: &mut App,
        handle: &Handle<GameDataAsset>,
    ) -> Result<(), gamedata::GameDataError> {
        const MAX_TICKS: u32 = 512;
        for _ in 0..MAX_TICKS {
            app.update();
            if app
                .world()
                .resource::<Assets<GameDataAsset>>()
                .get(handle)
                .is_some()
            {
                return Ok(());
            }

            let state = app
                .world()
                .resource::<AssetServer>()
                .load_state(handle.id());
            if let LoadState::Failed(err) = state {
                return Err(gamedata::GameDataError::Decode(format!(
                    "failed to load fixture table asset: {err}"
                )));
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Err(gamedata::GameDataError::Decode(
            "timed out waiting for fixture table asset".into(),
        ))
    }

    #[test]
    fn loads_gamedata_table_asset_from_catalog_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_fixture(temp.path());

        let mut app = App::new();
        app.add_plugins(MinimalPlugins).add_plugins(AssetPlugin {
            file_path: temp.path().to_string_lossy().into(),
            ..Default::default()
        });
        app.add_plugins(GameDataPlugin);

        let path = "sharedassets/test/fixturetable.aztbl";
        let handle = app
            .world()
            .resource::<AssetServer>()
            .load::<GameDataAsset>(path);

        await_asset(&mut app, &handle).expect("loaded table asset");

        let asset = app
            .world()
            .resource::<Assets<GameDataAsset>>()
            .get(&handle)
            .expect("loaded table asset");
        assert_eq!(asset.table.header().row_count, 1);
    }
}
