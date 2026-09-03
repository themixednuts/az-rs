//! Indexed read-only view over a decoded `GameData` table.

use std::borrow::Cow;

use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::GameDataError;
use crate::identity::{RowGuid, RowIndex};
use crate::table::body::{CellRef, TableBody};
use crate::table::encode::decode_body;

/// Read-only table snapshot with row/column indexes for runtime lookups.
///
/// String cells borrow directly from the backing table asset bytes; keep the table asset
/// alive for as long as the view is used.
#[derive(Debug, Clone, PartialEq)]
pub struct TableView<'a> {
    bytes: &'a [u8],
    body: Cow<'a, TableBody>,
    row_index_by_key_crc: FxHashMap<u32, usize>,
    row_index_by_guid: FxHashMap<RowGuid, usize>,
    row_index_by_name: FxHashMap<&'a str, usize>,
    column_index_by_crc: FxHashMap<u32, usize>,
}

impl<'a> TableView<'a> {
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self, GameDataError> {
        Self::from_body(bytes, Cow::Owned(decode_body(bytes)?))
    }

    pub fn from_parts(bytes: &'a [u8], body: &'a TableBody) -> Result<Self, GameDataError> {
        Self::from_body(bytes, Cow::Borrowed(body))
    }

    fn from_body(bytes: &'a [u8], body: Cow<'a, TableBody>) -> Result<Self, GameDataError> {
        let mut row_index_by_key_crc =
            FxHashMap::with_capacity_and_hasher(body.row_key_crcs.len(), FxBuildHasher);
        for (row_index, key_crc) in body.row_key_crcs.iter().copied().enumerate() {
            if key_crc == 0 {
                continue;
            }
            if row_index_by_key_crc.insert(key_crc, row_index).is_some() {
                return Err(GameDataError::Decode(format!(
                    "duplicate row key crc {key_crc} in GameData table"
                )));
            }
        }

        let mut row_index_by_guid =
            FxHashMap::with_capacity_and_hasher(body.row_guids.len(), FxBuildHasher);
        for (row_index, row_guid) in body.row_guids.iter().copied().enumerate() {
            if row_index_by_guid.insert(row_guid, row_index).is_some() {
                return Err(GameDataError::Decode(format!(
                    "duplicate row guid {row_guid} in GameData table"
                )));
            }
        }

        let mut row_index_by_name = FxHashMap::default();
        for (row_index, row_name) in body.row_names.iter().enumerate() {
            let Some(text) = row_name else {
                continue;
            };
            let Ok(name) = text.resolve(bytes) else {
                continue;
            };
            row_index_by_name.entry(name).or_insert(row_index);
        }

        let mut column_index_by_crc =
            FxHashMap::with_capacity_and_hasher(body.columns.len(), FxBuildHasher);
        for (column_index, column) in body.columns.iter().enumerate() {
            if column.crc == 0 {
                continue;
            }
            if column_index_by_crc
                .insert(column.crc, column_index)
                .is_some()
            {
                return Err(GameDataError::Decode(format!(
                    "duplicate column crc {} in GameData table",
                    column.crc
                )));
            }
        }

        Ok(Self {
            bytes,
            body,
            row_index_by_key_crc,
            row_index_by_guid,
            row_index_by_name,
            column_index_by_crc,
        })
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        self.bytes
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        self.body.row_key_crcs.len()
    }

    #[must_use]
    pub fn row_key_crc(&self, row_index: usize) -> Option<u32> {
        self.body.row_key_crcs.get(row_index).copied()
    }

    #[must_use]
    pub fn row_index_by_key_crc(&self, key_crc: u32) -> Option<usize> {
        self.row_index_by_key_crc.get(&key_crc).copied()
    }

    #[must_use]
    pub fn row_guid(&self, row_index: usize) -> Option<RowGuid> {
        self.body.row_guids.get(row_index).copied()
    }

    #[must_use]
    pub fn row_index_by_guid(&self, guid: RowGuid) -> Option<usize> {
        self.row_index_by_guid.get(&guid).copied()
    }

    #[must_use]
    pub fn row_index_by_name(&self, name: &str) -> Option<usize> {
        self.row_index_by_name.get(name).copied()
    }

    /// Converts a one-based [`RowIndex`] to a zero-based row position.
    #[must_use]
    pub const fn row_index_for(index: RowIndex) -> usize {
        index.zero_based() as usize
    }

    #[must_use]
    pub fn column_index_by_crc(&self, column_crc: u32) -> Option<usize> {
        self.column_index_by_crc.get(&column_crc).copied()
    }

    #[must_use]
    pub fn row_name(&self, row_index: usize) -> Option<&str> {
        self.body
            .row_names
            .get(row_index)?
            .as_ref()?
            .resolve(self.bytes)
            .ok()
    }

    #[must_use]
    pub fn cell(&self, row_index: usize, column_crc: u32) -> Option<CellRef> {
        let column_index = self.column_index_by_crc(column_crc)?;
        self.cell_at(row_index, column_index)
    }

    #[must_use]
    pub fn cell_at(&self, row_index: usize, column_index: usize) -> Option<CellRef> {
        *self.body.cells.get(column_index)?.get(row_index)?
    }

    #[must_use]
    pub fn cell_by_key_crc(&self, row_key_crc: u32, column_crc: u32) -> Option<CellRef> {
        let row_index = self.row_index_by_key_crc(row_key_crc)?;
        self.cell(row_index, column_crc)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn cell_as_str(&self, row_index: usize, column_crc: u32) -> Option<&'a str> {
        self.cell(row_index, column_crc)?.as_str(self.bytes)
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn cell_as_f64(&self, row_index: usize, column_crc: u32) -> Option<f64> {
        self.cell(row_index, column_crc)?.as_f64()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn cell_as_row_index(&self, row_index: usize, column_crc: u32) -> Option<RowIndex> {
        self.cell(row_index, column_crc)?.as_row_index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::RowIndex;
    use crate::release::SchemaHash;
    use crate::table::body::{CellType, CellValue, ColumnDescriptor, ScalarType};
    use crate::table::encode::{EncodeInput, EncodeRow, encode_table_asset, import_row_guid};

    fn sample_table_asset_bytes() -> Vec<u8> {
        encode_table_asset(&EncodeInput {
            table_name: "TestTable".into(),
            schema_hash: SchemaHash(0x0123_4567_89ab_cdef),
            table_name_crc: az_core::crc::Crc32::from_str_lower("TestTable").value(),
            row_type_crc: 0x3333_4444,
            columns: vec![
                ColumnDescriptor {
                    crc: 10,
                    cell_type: CellType::Scalar(ScalarType::String),
                    flags: 0,
                },
                ColumnDescriptor {
                    crc: 20,
                    cell_type: CellType::Scalar(ScalarType::F64),
                    flags: 0,
                },
            ],
            rows: vec![
                EncodeRow {
                    key_crc: 100,
                    debug_name: Some("alpha".into()),
                    row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 100),
                },
                EncodeRow {
                    key_crc: 200,
                    debug_name: Some("beta".into()),
                    row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 200),
                },
            ],
            cells: vec![
                vec![
                    Some(CellValue::string("alpha")),
                    Some(CellValue::string("beta")),
                ],
                vec![Some(CellValue::F64(1.5)), None],
            ],
            dependencies: Vec::new(),
        })
        .expect("sample table asset")
    }

    #[test]
    fn table_view_indexes_rows_and_columns() {
        let bytes = sample_table_asset_bytes();
        let view = TableView::from_bytes(&bytes).expect("build view");
        assert_eq!(view.row_count(), 2);
        assert_eq!(view.row_index_by_key_crc(100), Some(0));
        assert_eq!(view.row_index_by_key_crc(200), Some(1));
        assert_eq!(view.cell_as_str(0, 10), Some("alpha"));
        assert_eq!(
            view.cell_by_key_crc(100, 10)
                .and_then(|cell| cell.as_str(view.bytes())),
            Some("alpha")
        );
        assert_eq!(view.row_name(0), Some("alpha"));
        assert_eq!(view.row_name(1), Some("beta"));
        assert_eq!(view.cell_as_f64(0, 20), Some(1.5));
        assert_eq!(view.cell(1, 20), None);

        let alpha_guid = view.row_guid(0).expect("alpha guid");
        assert_eq!(view.row_index_by_guid(alpha_guid), Some(0));
        assert_eq!(view.row_index_by_name("alpha"), Some(0));
        assert_eq!(view.row_index_by_name("missing"), None);
        assert_eq!(
            TableView::row_index_for(RowIndex::from_one_based(1).expect("one-based")),
            0
        );
    }
}
