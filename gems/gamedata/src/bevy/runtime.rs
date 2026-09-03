//! System parameter for request-driven typed `GameData` access.

use std::marker::PhantomData;

use az_asset::AssetId;
use az_core::crc::Crc32;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use gamedata::{GameDataError, GameDataSchemaRow, SchemaTable, TableId};

use super::tables::GameDataTables;

#[derive(SystemParam)]
pub struct GameData<'w, 's> {
    tables: Res<'w, GameDataTables>,
    marker: PhantomData<&'s ()>,
}

impl GameData<'_, '_> {
    /// Access one physical table by its authored name.
    ///
    /// # Errors
    ///
    /// Returns any error the matching [`GameDataTables`] accessor returns: a
    /// loading error on the first request for a product or family, and a
    /// decode error when the catalog cannot resolve it or the loaded table
    /// does not satisfy `R`'s merged row schema.
    pub fn table<R>(&self, table_name: &str) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        self.tables.schema_table::<R>(table_name)
    }

    /// Access one physical table by the lowercase CRC of its name.
    ///
    /// # Errors
    ///
    /// Returns any error the matching [`GameDataTables`] accessor returns: a
    /// loading error on the first request for a product or family, and a
    /// decode error when the catalog cannot resolve it or the loaded table
    /// does not satisfy `R`'s merged row schema.
    pub fn table_by_crc<R>(
        &self,
        table_name_crc: impl Into<Crc32>,
    ) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        self.tables.schema_table_by_crc::<R>(table_name_crc)
    }

    /// Access one table by its exact product path in the active catalog.
    ///
    /// # Errors
    ///
    /// Returns any error the matching [`GameDataTables`] accessor returns: a
    /// loading error on the first request for a product or family, and a
    /// decode error when the catalog cannot resolve it or the loaded table
    /// does not satisfy `R`'s merged row schema.
    pub fn table_at<R>(&self, product_path: &str) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        self.tables.schema_table_at::<R>(product_path)
    }

    /// Access one table by its stable engine asset id.
    ///
    /// # Errors
    ///
    /// Returns any error the matching [`GameDataTables`] accessor returns: a
    /// loading error on the first request for a product or family, and a
    /// decode error when the catalog cannot resolve it or the loaded table
    /// does not satisfy `R`'s merged row schema.
    pub fn table_by_asset_id<R>(&self, asset_id: AssetId) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        self.tables.schema_table_by_asset_id::<R>(asset_id)
    }

    /// Access every physical table in one merged row-schema family.
    ///
    /// # Errors
    ///
    /// Returns any error the matching [`GameDataTables`] accessor returns: a
    /// loading error on the first request for a product or family, and a
    /// decode error when the catalog cannot resolve it or the loaded table
    /// does not satisfy `R`'s merged row schema.
    pub fn tables<R>(&self) -> Result<Vec<SchemaTable<R>>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        self.tables.schema_tables::<R>()
    }
}

/// Mutable lifecycle access for releasing cached `GameData` products.
///
/// Keep this separate from [`GameData`] so ordinary reads remain parallel.
#[derive(SystemParam)]
pub struct GameDataCache<'w, 's> {
    tables: ResMut<'w, GameDataTables>,
    marker: PhantomData<&'s ()>,
}

impl GameDataCache<'_, '_> {
    pub fn release_path(&mut self, product_path: &str) -> bool {
        self.tables.release_product(product_path)
    }

    pub fn release_table<R>(&mut self, table_id: TableId) -> bool
    where
        R: GameDataSchemaRow,
    {
        self.tables.release_table::<R>(table_id)
    }

    pub fn release_asset(&mut self, asset_id: AssetId) -> bool {
        self.tables.release_asset(asset_id)
    }

    pub fn release_schema<R>(&mut self)
    where
        R: GameDataSchemaRow,
    {
        self.tables.release_schema::<R>();
    }
}
