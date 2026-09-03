//! Runtime storage for compiled `GameData` table assets.
//!
//! A [`System`] owns the discovered physical tables. Callers select a physical
//! table by name and view it through one generated merged row schema with
//! [`System::schema_table`]. Authoring/import formats stay outside this path.

use std::any::{Any, TypeId};
use std::sync::{Arc, OnceLock, RwLock};

use az_core::crc::Crc32;
use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::GameDataError;
use crate::table::{CellRef, Row, TableAsset, TableBody};

mod schema_table;
mod table_id;

pub use schema_table::{SchemaRow, SchemaRowRef, SchemaTable, SchemaValue};
pub use table_id::TableId;

/// Runtime owner of every catalog-discovered `GameData` table product.
#[derive(Debug, Default)]
pub struct System {
    tables: Vec<Slot>,
}

#[derive(Debug)]
pub(super) struct Slot {
    key: u64,
    asset: Arc<TableAsset>,
    body: OnceLock<TableBody>,
    index: OnceLock<Index>,
    projections: RwLock<FxHashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

#[derive(Debug)]
struct Index {
    rows: FxHashMap<u32, u32>,
    columns: FxHashMap<u32, u16>,
}

/// Erased physical table used only while materializing a generated row schema.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct AnyTableRef<'a> {
    pub(super) slot: &'a Slot,
    pub(super) logical_name: &'a str,
}

/// Erased column used only by generated row-schema decoding.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct AnyColumnSlot {
    pub(super) index: u16,
    pub(super) column: &'static str,
}

/// Erased row used only by generated row-schema decoding.
#[doc(hidden)]
#[derive(Debug, Clone, Copy)]
pub struct AnyRowRef<'a> {
    pub(super) table: AnyTableRef<'a>,
    pub(super) row: u32,
}

impl System {
    /// Insert one catalog-discovered compiled table product.
    ///
    /// # Errors
    ///
    /// Returns an error if another product has the same physical-table and row
    /// schema identity.
    pub fn insert(&mut self, asset: impl Into<Arc<TableAsset>>) -> Result<(), GameDataError> {
        let asset = asset.into();
        let header = asset.header();
        let key = table_key(header.table_name_crc, header.row_type_crc);
        match self.tables.binary_search_by_key(&key, |slot| slot.key) {
            Ok(_) => Err(GameDataError::Decode(format!(
                "duplicate GameData table identity table_crc={:#010x} row_crc={:#010x}",
                header.table_name_crc, header.row_type_crc
            ))),
            Err(index) => {
                self.tables.insert(
                    index,
                    Slot {
                        key,
                        asset,
                        body: OnceLock::new(),
                        index: OnceLock::new(),
                        projections: RwLock::default(),
                    },
                );
                Ok(())
            }
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.tables.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Remove every loaded table with the requested merged row identity.
    pub fn remove_row_type(&mut self, row_type_crc: u32) -> usize {
        let previous = self.tables.len();
        self.tables
            .retain(|slot| slot.asset.header().row_type_crc != row_type_crc);
        previous - self.tables.len()
    }

    /// Remove one loaded physical table and all of its cached projections.
    pub fn remove_table(&mut self, table_name_crc: u32, row_type_crc: u32) -> bool {
        let key = table_key(table_name_crc, row_type_crc);
        let Ok(index) = self.tables.binary_search_by_key(&key, |slot| slot.key) else {
            return false;
        };
        self.tables.remove(index);
        true
    }

    /// Snapshot loaded headers in deterministic physical-table identity order.
    #[must_use]
    pub fn table_header_snapshot(&self) -> Vec<(crate::SchemaHash, u32, u32, u32)> {
        self.tables
            .iter()
            .map(|slot| {
                let header = slot.asset.header();
                (
                    header.schema_hash,
                    header.table_name_crc,
                    header.row_type_crc,
                    header.row_count,
                )
            })
            .collect()
    }

    /// View one physical table through its generated merged row schema.
    ///
    /// # Errors
    ///
    /// Returns an error if the named table is absent or its physical columns do
    /// not satisfy the selected merged row schema.
    pub fn schema_table<R>(&self, table_name: &str) -> Result<SchemaTable<R>, GameDataError>
    where
        R: crate::GameDataSchemaRow,
    {
        let table_name_crc = Crc32::from_str_lower(table_name).value();
        let row_type_crc = <R as Row>::CRC;
        self.slot_by_key(table_key(table_name_crc, row_type_crc), table_name)?
            .schema_table::<R>()
    }

    /// View one physical table selected by lowercase name CRC.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::MissingTable`] when no loaded table matches
    /// both `table_name_crc` and `R`'s row-type CRC, or
    /// [`GameDataError::Decode`] when that table's body or column index cannot
    /// be decoded or does not satisfy `R`'s merged row schema.
    pub fn schema_table_by_crc<R>(
        &self,
        table_name_crc: u32,
    ) -> Result<SchemaTable<R>, GameDataError>
    where
        R: crate::GameDataSchemaRow,
    {
        let row_type_crc = <R as Row>::CRC;
        self.slot_by_key(
            table_key(table_name_crc, row_type_crc),
            &format!("crc:{table_name_crc:#010x}"),
        )?
        .schema_table::<R>()
    }

    /// View every loaded physical table belonging to one merged row schema.
    ///
    /// Physical names come from the compiled products. The result follows the
    /// deterministic product identity order used by [`System`].
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] for the first matching table whose
    /// body or column index cannot be decoded or whose physical columns do not
    /// satisfy `R`'s merged row schema. Matching no table at all is an empty
    /// result, not an error.
    pub fn schema_tables<R>(&self) -> Result<Vec<SchemaTable<R>>, GameDataError>
    where
        R: crate::GameDataSchemaRow,
    {
        let row_type_crc = <R as Row>::CRC;
        self.tables
            .iter()
            .filter(|slot| slot.asset.header().row_type_crc == row_type_crc)
            .map(Slot::schema_table::<R>)
            .collect()
    }

    fn slot_by_key(&self, key: u64, logical_name: &str) -> Result<&Slot, GameDataError> {
        self.tables
            .binary_search_by_key(&key, |slot| slot.key)
            .ok()
            .and_then(|index| self.tables.get(index))
            .ok_or_else(|| GameDataError::MissingTable {
                logical_name: logical_name.to_owned(),
            })
    }
}

impl<'a> AnyTableRef<'a> {
    pub(super) const fn logical_name(self) -> &'a str {
        self.logical_name
    }

    pub(super) fn bytes(self) -> &'a [u8] {
        self.slot.asset.bytes()
    }

    pub(super) fn body(self) -> Result<&'a TableBody, GameDataError> {
        self.slot.body()
    }

    pub(super) fn len(self) -> usize {
        self.slot.asset.header().row_count as usize
    }

    pub(super) fn cell_ref_at(self, row: u32, column: u16) -> Option<CellRef> {
        *self
            .slot
            .body()
            .ok()?
            .cells
            .get(column as usize)?
            .get(row as usize)?
    }

    pub(super) fn rows(self) -> impl ExactSizeIterator<Item = AnyRowRef<'a>> {
        let count = self.slot.asset.header().row_count;
        (0..count).map(move |row| AnyRowRef { table: self, row })
    }
}

impl AnyRowRef<'_> {
    pub(super) const fn zero_based_index(self) -> u32 {
        self.row
    }
}

pub(super) fn read_list_count(
    data: &mut &[u8],
    table: &str,
    column: &'static str,
) -> Result<usize, GameDataError> {
    let bytes = data.get(..4).ok_or_else(|| {
        GameDataError::Decode(format!(
            "table `{table}` column `{column}` list payload truncated while reading count"
        ))
    })?;
    *data = &data[4..];
    usize::try_from(u32::from_le_bytes(bytes.try_into().expect("slice length"))).map_err(|_| {
        GameDataError::Decode(format!(
            "table `{table}` column `{column}` list count exceeds usize"
        ))
    })
}

impl Slot {
    fn schema_table<R>(&self) -> Result<SchemaTable<R>, GameDataError>
    where
        R: crate::GameDataSchemaRow,
    {
        let type_id = TypeId::of::<R>();
        if let Some(cached) = self
            .projections
            .read()
            .expect("GameData projection cache lock poisoned")
            .get(&type_id)
            .cloned()
        {
            return SchemaTable::from_cached(cached);
        }

        let table = SchemaTable::<R>::materialize(AnyTableRef {
            slot: self,
            logical_name: self.asset.table_name(),
        })?;
        let cached: Arc<dyn Any + Send + Sync> = table;
        let projection = self
            .projections
            .write()
            .expect("GameData projection cache lock poisoned")
            .entry(type_id)
            .or_insert(cached)
            .clone();
        SchemaTable::from_cached(projection)
    }

    fn body(&self) -> Result<&TableBody, GameDataError> {
        if let Some(body) = self.body.get() {
            return Ok(body);
        }
        let body = crate::table::encode::decode_body(self.asset.bytes())?;
        let _ = self.body.set(body);
        Ok(self.body.get().expect("body was just initialized"))
    }

    fn index(&self) -> Result<&Index, GameDataError> {
        if let Some(index) = self.index.get() {
            return Ok(index);
        }
        let index = Index::from_body(self.body()?)?;
        let _ = self.index.set(index);
        Ok(self.index.get().expect("index was just initialized"))
    }
}

impl Index {
    fn from_body(body: &TableBody) -> Result<Self, GameDataError> {
        let mut rows = FxHashMap::with_capacity_and_hasher(body.row_key_crcs.len(), FxBuildHasher);
        for (row, key_crc) in body.row_key_crcs.iter().copied().enumerate() {
            if key_crc == 0 {
                continue;
            }
            let row = u32::try_from(row)
                .map_err(|_| GameDataError::Decode("GameData row index exceeds u32".to_owned()))?;
            if rows.insert(key_crc, row).is_some() {
                return Err(GameDataError::Decode(format!(
                    "duplicate GameData row key CRC {key_crc:#010x}"
                )));
            }
        }

        let mut columns = FxHashMap::with_capacity_and_hasher(body.columns.len(), FxBuildHasher);
        for (column, descriptor) in body.columns.iter().enumerate() {
            if descriptor.crc == 0 {
                continue;
            }
            let column = u16::try_from(column).map_err(|_| {
                GameDataError::Decode("GameData column index exceeds u16".to_owned())
            })?;
            if columns.insert(descriptor.crc, column).is_some() {
                return Err(GameDataError::Decode(format!(
                    "duplicate GameData column CRC {:#010x}",
                    descriptor.crc
                )));
            }
        }

        Ok(Self { rows, columns })
    }
}

const fn table_key(table_crc: u32, row_crc: u32) -> u64 {
    ((table_crc as u64) << 32) | row_crc as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::SchemaHash;
    use crate::table::encode::{
        CellValue, ColumnDescriptor, EncodeInput, EncodeRow, encode_table_asset, import_row_guid,
    };
    use crate::table::{CellType, ScalarType};

    struct TestRow;

    impl Row for TestRow {
        const NAME: &'static str = "TestRow";
    }

    impl crate::GameDataRow for TestRow {
        const KEY_FIELD_NAMES: &'static [&'static str] = &[];
    }

    impl crate::GameDataSchemaRow for TestRow {
        const SCHEMA: crate::RowSchemaDescriptor = crate::RowSchemaDescriptor::new(Self::NAME, &[]);

        fn decode(_row: crate::SchemaRowRef<'_, '_, Self>) -> Result<Self, GameDataError> {
            Ok(Self)
        }
    }

    fn test_asset(table_name: &'static str) -> TableAsset {
        let table_crc = Crc32::from_str_lower(table_name).value();
        let key_crc = Crc32::from_str_lower("Mercenary").value();
        let bytes = encode_table_asset(&EncodeInput {
            table_name: table_name.into(),
            schema_hash: SchemaHash(0x1234),
            table_name_crc: table_crc,
            row_type_crc: TestRow::CRC,
            columns: vec![ColumnDescriptor {
                crc: Crc32::from_str_lower("Key").value(),
                cell_type: CellType::Scalar(ScalarType::RowKey),
                flags: 0,
            }],
            rows: vec![EncodeRow {
                key_crc,
                debug_name: Some("Mercenary".into()),
                row_guid: import_row_guid(table_crc, TestRow::CRC, key_crc),
            }],
            cells: vec![vec![Some(CellValue::row_key("Mercenary"))]],
            dependencies: Vec::new(),
        })
        .expect("encode test table");
        TableAsset::from_bytes(bytes).expect("compiled table asset")
    }

    #[test]
    fn rejects_duplicate_physical_table_identity() {
        let mut system = System::default();
        system.insert(test_asset("TestTable")).expect("first table");
        let error = system
            .insert(test_asset("TestTable"))
            .expect_err("duplicate table");
        assert!(
            error
                .to_string()
                .contains("duplicate GameData table identity")
        );
    }

    #[test]
    fn header_snapshot_is_sorted_by_physical_identity() {
        let mut system = System::default();
        system.insert(test_asset("ZTable")).expect("z table");
        system.insert(test_asset("ATable")).expect("a table");
        let headers = system.table_header_snapshot();
        assert_eq!(headers.len(), 2);
        assert!(headers[0].1 < headers[1].1);
    }

    #[test]
    fn discovers_every_physical_table_for_a_merged_row_schema() {
        let mut system = System::default();
        system
            .insert(test_asset("SecondTable"))
            .expect("second table");
        system
            .insert(test_asset("FirstTable"))
            .expect("first table");

        let tables = system.schema_tables::<TestRow>().expect("schema family");
        let mut names = tables.iter().map(SchemaTable::name).collect::<Vec<_>>();
        names.sort_unstable();

        assert_eq!(names, ["FirstTable", "SecondTable"]);
    }
}
