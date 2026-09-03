use az_asset::AssetId;
use thiserror::Error;

use crate::release::{GameDataReleaseId, SchemaHash};

#[derive(Debug, Error)]
pub enum GameDataError {
    #[error("required game-data table `{logical_name}` is missing")]
    MissingTable { logical_name: String },

    #[error("game-data table family `{schema}` is loading")]
    LoadingTableFamily { schema: String },

    #[error("game-data product `{path}` is loading")]
    LoadingTableProduct { path: String },

    #[error("game-data table family `{schema}` failed to load: {error}")]
    TableFamilyLoadFailed { schema: String, error: String },

    #[error("asset {asset_id} has schema {actual}, expected {expected}")]
    SchemaMismatch {
        asset_id: AssetId,
        expected: SchemaHash,
        actual: SchemaHash,
    },

    #[error("invalid foreign key from `{table}` row `{row}` to `{target_table}`")]
    InvalidForeignKey {
        table: &'static str,
        row: String,
        target_table: &'static str,
    },

    #[error("game-data release mismatch: expected {expected}, actual {actual}")]
    ReleaseMismatch {
        expected: GameDataReleaseId,
        actual: GameDataReleaseId,
    },

    #[error("game-data decode failed: {0}")]
    Decode(String),
}

impl GameDataError {
    #[must_use]
    pub const fn is_loading(&self) -> bool {
        matches!(
            self,
            Self::LoadingTableFamily { .. } | Self::LoadingTableProduct { .. }
        )
    }
}
