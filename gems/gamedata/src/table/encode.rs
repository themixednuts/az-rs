//! Sectioned `GameData` table asset encode/decode (ADR 0002 cold/hot split).

use bytes::Buf;
#[cfg(any(feature = "authoring", test))]
use bytes::BufMut;
#[cfg(any(feature = "authoring", test))]
use std::borrow::Cow;

use crate::GameDataError;
use crate::format::TableSectionId;
#[cfg(any(feature = "authoring", test))]
use crate::identity::RowGuid;
#[cfg(any(feature = "authoring", test))]
use crate::release::SchemaHash;
use crate::table::asset::TableHeader;
use crate::table::body::TableBody;
use crate::table::section::ParsedSectionFile;
#[cfg(any(feature = "authoring", test))]
use crate::table::section::{FLAG_LITTLE_ENDIAN, build_section_file};

mod cell;
mod metadata;
mod pool;
mod scalar;
mod text;

pub use crate::table::body::ColumnDescriptor;
#[cfg(any(feature = "authoring", test))]
pub use crate::table::body::{CellValue, TableDependency};
#[cfg(feature = "authoring")]
pub use crate::table::body::{DEPENDENCY_KIND_FOREIGN_KEY, ForeignKeyValue, ListValue};
pub use cell::{read_atom_cell_ref, read_list_element_cell_ref, read_list_element_type};
#[cfg(any(feature = "authoring", test))]
pub use metadata::{import_row_guid, import_row_guid_with_name};

use cell::{read_cell_ref, read_cell_type};
#[cfg(any(feature = "authoring", test))]
use cell::{write_cell_type, write_cell_value};
use metadata::{
    decode_debug_names_section, decode_dependency_index_section, decode_row_guids_section,
    decode_row_key_aliases_section, decode_schema_section,
};
#[cfg(any(feature = "authoring", test))]
use metadata::{
    encode_debug_names_section, encode_dependency_index_section, encode_row_guids_section,
    encode_row_key_aliases_section, encode_schema_section,
};
#[cfg(any(feature = "authoring", test))]
use pool::{StringPoolBuilder, collect_string_pool};
use scalar::{read_u8, read_u32};
use text::string_pool_base;

#[cfg(any(feature = "authoring", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeRow<'a> {
    pub key_crc: u32,
    pub debug_name: Option<Cow<'a, str>>,
    pub row_guid: RowGuid,
}

#[cfg(any(feature = "authoring", test))]
#[derive(Debug, Clone, PartialEq)]
pub struct EncodeInput<'a> {
    pub table_name: Cow<'a, str>,
    pub schema_hash: SchemaHash,
    pub table_name_crc: u32,
    pub row_type_crc: u32,
    pub columns: Vec<ColumnDescriptor>,
    pub rows: Vec<EncodeRow<'a>>,
    pub cells: Vec<Vec<Option<CellValue<'a>>>>,
    pub dependencies: Vec<TableDependency>,
}

/// Encodes one physical table into the sectioned `AZTBL` byte layout.
///
/// # Errors
///
/// Returns [`GameDataError::Decode`] when `table_name` is empty or its
/// lowercase CRC disagrees with `table_name_crc`, when the cell grid is not
/// one column vector per declared column or one entry per row, when a cell
/// value does not match its column's declared cell type, and when the column
/// count, row count, string pool, or an encoded section payload exceeds the
/// `u32` fields the format stores them in.
#[cfg(any(feature = "authoring", test))]
pub fn encode_table_asset(input: &EncodeInput<'_>) -> Result<Vec<u8>, GameDataError> {
    if input.table_name.is_empty() {
        return Err(GameDataError::Decode(
            "GameData physical table name must not be empty".into(),
        ));
    }
    let actual_table_name_crc = az_core::crc::Crc32::from_str_lower(&input.table_name).value();
    if actual_table_name_crc != input.table_name_crc {
        return Err(GameDataError::Decode(format!(
            "GameData physical table `{}` has lowercase CRC {actual_table_name_crc:#010x}, expected {:#010x}",
            input.table_name, input.table_name_crc,
        )));
    }
    let column_count = u32::try_from(input.columns.len())
        .map_err(|_| GameDataError::Decode("column_count exceeds u32".into()))?;
    if input.cells.len() != input.columns.len() {
        return Err(GameDataError::Decode(format!(
            "cells column count {} does not match schema column count {}",
            input.cells.len(),
            input.columns.len()
        )));
    }
    for (column_index, column_cells) in input.cells.iter().enumerate() {
        if column_cells.len() != input.rows.len() {
            return Err(GameDataError::Decode(format!(
                "cells[{column_index}] row count {} does not match table row count {}",
                column_cells.len(),
                input.rows.len()
            )));
        }
    }

    let schema_payload = encode_schema_section(
        input.schema_hash,
        input.table_name_crc,
        input.row_type_crc,
        column_count,
    );
    let string_pool = collect_string_pool(input)?;
    let use_string_pool = !string_pool.is_empty();
    let hot_payload = encode_hot_columns_section(input, &string_pool, use_string_pool)?;
    let guid_payload = encode_row_guids_section(&input.rows);
    let names_payload = encode_debug_names_section(&input.rows, &string_pool, use_string_pool);
    let string_pool_payload = string_pool.finish();
    let alias_payload = encode_row_key_aliases_section(&input.rows)?;
    let dependency_payload = encode_dependency_index_section(&input.dependencies);

    let schema_hash_tag = u32::try_from(input.schema_hash.0 & 0xffff_ffff).unwrap_or(u32::MAX);
    let table_identity_payload = encode_table_identity_section(&input.table_name)?;
    let mut sections = vec![
        (
            TableSectionId::Schema as u32,
            schema_payload,
            schema_hash_tag,
        ),
        (TableSectionId::HotColumns as u32, hot_payload, 0),
        (TableSectionId::RowGuids as u32, guid_payload, 0),
        (
            TableSectionId::TableIdentity as u32,
            table_identity_payload,
            input.table_name_crc,
        ),
    ];
    if let Some(string_pool_payload) = string_pool_payload {
        sections.push((TableSectionId::StringPool as u32, string_pool_payload, 0));
    }
    sections.push((TableSectionId::RowKeyAliases as u32, alias_payload, 0));
    if let Some(names_payload) = names_payload {
        sections.push((TableSectionId::DebugNames as u32, names_payload, 0));
    }
    if let Some(dependency_payload) = dependency_payload {
        sections.push((
            TableSectionId::DependencyIndex as u32,
            dependency_payload,
            0,
        ));
    }

    build_section_file(FLAG_LITTLE_ENDIAN, &sections)
}

#[cfg(any(feature = "authoring", test))]
fn encode_table_identity_section(table_name: &str) -> Result<Vec<u8>, GameDataError> {
    let name_len = u32::try_from(table_name.len())
        .map_err(|_| GameDataError::Decode("GameData physical table name exceeds u32".into()))?;
    let mut bytes = Vec::with_capacity(4 + table_name.len());
    bytes.put_u32_le(name_len);
    bytes.extend_from_slice(table_name.as_bytes());
    Ok(bytes)
}

pub fn decode_table_name(bytes: &[u8]) -> Result<Box<str>, GameDataError> {
    let section_file = ParsedSectionFile::parse(bytes)?;
    let mut data = section_file.section_payload(bytes, TableSectionId::TableIdentity as u32)?;
    let name_len = read_u32(&mut data, "table_identity.name_len")? as usize;
    let name = data.get(..name_len).ok_or_else(|| {
        GameDataError::Decode(format!(
            "TABLE_IDENTITY name length {name_len} exceeds remaining {} bytes",
            data.len(),
        ))
    })?;
    data.advance(name_len);
    if data.remaining() != 0 {
        return Err(GameDataError::Decode(format!(
            "TABLE_IDENTITY section has {} trailing byte(s)",
            data.remaining(),
        )));
    }
    let name = std::str::from_utf8(name).map_err(|error| {
        GameDataError::Decode(format!("TABLE_IDENTITY name is not UTF-8: {error}"))
    })?;
    if name.is_empty() {
        return Err(GameDataError::Decode(
            "TABLE_IDENTITY physical table name must not be empty".into(),
        ));
    }
    Ok(name.into())
}

pub fn decode_body(bytes: &[u8]) -> Result<TableBody, GameDataError> {
    let section_file = ParsedSectionFile::parse(bytes)?;

    let schema = section_file.section_payload(bytes, TableSectionId::Schema as u32)?;
    let hot = section_file.section_payload(bytes, TableSectionId::HotColumns as u32)?;
    let guids = section_file.section_payload(bytes, TableSectionId::RowGuids as u32)?;
    let aliases = section_file.section_payload(bytes, TableSectionId::RowKeyAliases as u32)?;
    let names = section_file
        .section_payload(bytes, TableSectionId::DebugNames as u32)
        .ok();
    let dependencies = section_file
        .section_payload(bytes, TableSectionId::DependencyIndex as u32)
        .ok();
    let pool_base = string_pool_base(&section_file, bytes)?;

    let (_schema_hash, _table_name_crc, _row_type_crc, column_count) =
        decode_schema_section(schema)?;
    let mut data = hot;
    let row_count = read_u32(&mut data, "hot.row_count")?;

    let mut columns = Vec::with_capacity(column_count as usize);
    for index in 0..column_count {
        columns.push(ColumnDescriptor {
            crc: read_u32(&mut data, &format!("hot.column[{index}].crc"))?,
            cell_type: read_cell_type(&mut data, &format!("hot.column[{index}].cell_type"))?,
            flags: read_u32(&mut data, &format!("hot.column[{index}].flags"))?,
        });
    }

    let mut cells = vec![vec![None; row_count as usize]; column_count as usize];
    for (column_index, column) in columns.iter().enumerate() {
        for (row_index, cell) in cells[column_index].iter_mut().enumerate() {
            let present = read_u8(
                &mut data,
                &format!("hot.cells[{column_index}][{row_index}].present"),
            )?;
            if present == 0 {
                continue;
            }
            if present != 1 {
                return Err(GameDataError::Decode(format!(
                    "hot.cells[{column_index}][{row_index}] expected present flag 0 or 1, got {present}"
                )));
            }
            *cell = Some(read_cell_ref(
                bytes,
                &mut data,
                column.cell_type,
                column_index,
                row_index,
                pool_base,
            )?);
        }
    }
    if data.remaining() != 0 {
        return Err(GameDataError::Decode(format!(
            "HOT_COLUMNS section has {} trailing byte(s)",
            data.remaining()
        )));
    }

    let row_guids = decode_row_guids_section(guids, row_count)?;
    let alias_map = decode_row_key_aliases_section(aliases)?;
    let row_names = names
        .map(|payload| decode_debug_names_section(bytes, payload, row_count, pool_base))
        .transpose()?
        .unwrap_or_else(|| vec![None; row_count as usize]);
    let dependencies = dependencies
        .map(decode_dependency_index_section)
        .transpose()?
        .unwrap_or_default();

    let mut row_key_crcs = vec![0u32; row_count as usize];
    for (key_crc, row_index) in alias_map {
        let slot = row_index
            .zero_based()
            .try_into()
            .ok()
            .and_then(|index: usize| row_key_crcs.get_mut(index));
        if let Some(slot) = slot {
            if *slot != 0 && *slot != key_crc {
                return Err(GameDataError::Decode(format!(
                    "duplicate row key alias target for row index {}",
                    row_index.one_based()
                )));
            }
            *slot = key_crc;
        }
    }

    Ok(TableBody {
        columns,
        row_key_crcs,
        row_guids,
        row_names,
        cells,
        dependencies,
    })
}

pub fn parse_header(bytes: &[u8]) -> Result<TableHeader, GameDataError> {
    let section_file = ParsedSectionFile::parse(bytes)?;
    let schema = section_file.section_payload(bytes, TableSectionId::Schema as u32)?;
    let hot = section_file.section_payload(bytes, TableSectionId::HotColumns as u32)?;
    let (schema_hash, table_name_crc, row_type_crc, column_count) = decode_schema_section(schema)?;
    let mut data = hot;
    let row_count = read_u32(&mut data, "hot.row_count")?;
    Ok(TableHeader {
        flags: section_file.flags,
        schema_hash,
        table_name_crc,
        row_type_crc,
        row_count,
        column_count,
    })
}

#[cfg(test)]
pub fn fixture_table_asset_bytes(
    schema_hash: SchemaHash,
    table_name: &str,
    row_type_crc: u32,
    row_count: u32,
) -> Vec<u8> {
    let table_name_crc = az_core::crc::Crc32::from_str_lower(table_name).value();
    let rows = (0..row_count)
        .map(|index| EncodeRow {
            key_crc: index + 1,
            debug_name: None,
            row_guid: import_row_guid(table_name_crc, row_type_crc, index + 1),
        })
        .collect();
    encode_table_asset(&EncodeInput {
        table_name: Cow::Borrowed(table_name),
        schema_hash,
        table_name_crc,
        row_type_crc,
        columns: vec![],
        rows,
        cells: vec![],
        dependencies: Vec::new(),
    })
    .expect("minimal table asset")
}

#[cfg(any(feature = "authoring", test))]
fn encode_hot_columns_section(
    input: &EncodeInput<'_>,
    string_pool: &StringPoolBuilder,
    use_string_pool: bool,
) -> Result<Vec<u8>, GameDataError> {
    let mut bytes = Vec::new();
    bytes.put_u32_le(u32::try_from(input.rows.len()).expect("checked"));
    for column in &input.columns {
        bytes.put_u32_le(column.crc);
        write_cell_type(&mut bytes, column.cell_type);
        bytes.put_u32_le(column.flags);
    }
    for (column_index, column_cells) in input.cells.iter().enumerate() {
        let column = input
            .columns
            .get(column_index)
            .expect("cells column count checked");
        for (row_index, cell) in column_cells.iter().enumerate() {
            let Some(cell) = cell else {
                bytes.put_u8(0);
                continue;
            };
            bytes.put_u8(1);
            write_cell_value(
                &mut bytes,
                cell,
                column.cell_type,
                string_pool,
                use_string_pool,
            )
            .map_err(|err| {
                GameDataError::Decode(format!(
                    "hot.cells[{column_index}][{row_index}] encode failed: {err}"
                ))
            })?;
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::num::{NonZeroI8, NonZeroI16, NonZeroU8, NonZeroU16};

    use super::*;
    use crate::identity::RowIndex;
    use crate::table::body::{CellType, DEPENDENCY_KIND_FOREIGN_KEY, ForeignKeyValue, ScalarType};

    const SINGLE_SCALAR_CELL_HOT_PREFIX_LEN: usize = 4 + 4 + 2 + 4 + 1;
    const TEST_TABLE_NAME: &str = "TestTable";

    fn test_table_crc() -> u32 {
        az_core::crc::Crc32::from_str_lower(TEST_TABLE_NAME).value()
    }

    fn encoded_single_cell_payload_len(cell_type: CellType, cell: CellValue<'_>) -> usize {
        let input = EncodeInput {
            table_name: Cow::Borrowed(TEST_TABLE_NAME),
            schema_hash: SchemaHash(1),
            table_name_crc: test_table_crc(),
            row_type_crc: 0x3333_4444,
            columns: vec![ColumnDescriptor {
                crc: 10,
                cell_type,
                flags: 0,
            }],
            rows: vec![EncodeRow {
                key_crc: 100,
                debug_name: None,
                row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 100),
            }],
            cells: vec![vec![Some(cell)]],
            dependencies: Vec::new(),
        };
        let bytes = encode_table_asset(&input).expect("encode");
        let section_file = ParsedSectionFile::parse(&bytes).expect("parse sections");
        let hot = section_file
            .section_payload(&bytes, TableSectionId::HotColumns as u32)
            .expect("hot section");
        assert_eq!(
            hot.get(SINGLE_SCALAR_CELL_HOT_PREFIX_LEN - 1).copied(),
            Some(1),
            "{cell_type:?} cell should be present"
        );
        decode_body(&bytes).expect("decode");
        hot.len() - SINGLE_SCALAR_CELL_HOT_PREFIX_LEN
    }

    #[test]
    fn sectioned_roundtrip_preserves_guids_and_aliases() {
        let input = EncodeInput {
            table_name: Cow::Borrowed(TEST_TABLE_NAME),
            schema_hash: SchemaHash(0x0123_4567_89ab_cdef),
            table_name_crc: test_table_crc(),
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
        };
        let bytes = encode_table_asset(&input).expect("encode sectioned");
        let body = decode_body(&bytes).expect("decode sectioned");
        let view = crate::table::view::TableView::from_bytes(&bytes).expect("view");
        assert_eq!(body.row_key_crcs, vec![100, 200]);
        assert_eq!(
            body.row_guids,
            input
                .rows
                .iter()
                .map(|row| row.row_guid)
                .collect::<Vec<_>>()
        );
        assert_eq!(view.row_name(0), Some("alpha"));
        assert_eq!(view.row_name(1), Some("beta"));
        assert_eq!(view.cell_as_str(0, 10), Some("alpha"));
    }

    #[test]
    fn row_index_column_roundtrips() {
        let index = RowIndex::from_one_based(2).expect("row index");
        let input = EncodeInput {
            table_name: Cow::Borrowed(TEST_TABLE_NAME),
            schema_hash: SchemaHash(1),
            table_name_crc: test_table_crc(),
            row_type_crc: 0x3333_4444,
            columns: vec![ColumnDescriptor {
                crc: 30,
                cell_type: CellType::Scalar(ScalarType::RowIndex),
                flags: 0,
            }],
            rows: vec![EncodeRow {
                key_crc: 100,
                debug_name: None,
                row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 100),
            }],
            cells: vec![vec![Some(CellValue::RowIndex(index))]],
            dependencies: Vec::new(),
        };
        let bytes = encode_table_asset(&input).expect("encode");
        let view = crate::table::view::TableView::from_bytes(&bytes).expect("view");
        assert_eq!(view.cell_as_row_index(0, 30), Some(index));
    }

    #[test]
    fn narrow_integer_columns_use_fixed_width_payloads() {
        let cases = [
            (CellType::Scalar(ScalarType::I8), CellValue::I8(i8::MIN), 1),
            (CellType::Scalar(ScalarType::U8), CellValue::U8(u8::MAX), 1),
            (
                CellType::Scalar(ScalarType::I16),
                CellValue::I16(i16::MIN),
                2,
            ),
            (
                CellType::Scalar(ScalarType::U16),
                CellValue::U16(u16::MAX),
                2,
            ),
            (
                CellType::Scalar(ScalarType::NonZeroI8),
                CellValue::NonZeroI8(NonZeroI8::new(i8::MIN).expect("nonzero")),
                1,
            ),
            (
                CellType::Scalar(ScalarType::NonZeroU8),
                CellValue::NonZeroU8(NonZeroU8::new(u8::MAX).expect("nonzero")),
                1,
            ),
            (
                CellType::Scalar(ScalarType::NonZeroI16),
                CellValue::NonZeroI16(NonZeroI16::new(i16::MIN).expect("nonzero")),
                2,
            ),
            (
                CellType::Scalar(ScalarType::NonZeroU16),
                CellValue::NonZeroU16(NonZeroU16::new(u16::MAX).expect("nonzero")),
                2,
            ),
        ];

        for (cell_type, cell, expected_len) in cases {
            assert_eq!(
                encoded_single_cell_payload_len(cell_type, cell),
                expected_len,
                "{cell_type:?} should use its exact fixed width"
            );
        }
    }

    #[test]
    fn duplicate_row_key_aliases_are_not_indexed() {
        let input = EncodeInput {
            table_name: Cow::Borrowed(TEST_TABLE_NAME),
            schema_hash: SchemaHash(1),
            table_name_crc: test_table_crc(),
            row_type_crc: 0x3333_4444,
            columns: vec![ColumnDescriptor {
                crc: 10,
                cell_type: CellType::Scalar(ScalarType::String),
                flags: 0,
            }],
            rows: vec![
                EncodeRow {
                    key_crc: 100,
                    debug_name: Some("alpha".into()),
                    row_guid: import_row_guid_with_name(0x1111_2222, 0x3333_4444, "alpha"),
                },
                EncodeRow {
                    key_crc: 100,
                    debug_name: Some("beta".into()),
                    row_guid: import_row_guid_with_name(0x1111_2222, 0x3333_4444, "beta"),
                },
            ],
            cells: vec![vec![
                Some(CellValue::string("alpha")),
                Some(CellValue::string("beta")),
            ]],
            dependencies: Vec::new(),
        };

        let bytes = encode_table_asset(&input).expect("encode");
        let body = decode_body(&bytes).expect("decode");
        let view = crate::table::view::TableView::from_bytes(&bytes).expect("view");
        assert_eq!(body.row_key_crcs, vec![0, 0]);
        assert_ne!(body.row_guids[0], body.row_guids[1]);
        assert_eq!(view.row_index_by_key_crc(100), None);
        assert_eq!(view.row_index_by_name("alpha"), Some(0));
        assert_eq!(view.row_index_by_name("beta"), Some(1));
    }

    #[test]
    fn string_pool_deduplicates_shared_cell_and_debug_names() {
        let input = EncodeInput {
            table_name: Cow::Borrowed(TEST_TABLE_NAME),
            schema_hash: SchemaHash(1),
            table_name_crc: test_table_crc(),
            row_type_crc: 0x3333_4444,
            columns: vec![ColumnDescriptor {
                crc: 10,
                cell_type: CellType::Scalar(ScalarType::String),
                flags: 0,
            }],
            rows: vec![
                EncodeRow {
                    key_crc: 100,
                    debug_name: Some("shared".into()),
                    row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 100),
                },
                EncodeRow {
                    key_crc: 200,
                    debug_name: Some("shared".into()),
                    row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 200),
                },
            ],
            cells: vec![vec![
                Some(CellValue::string("shared")),
                Some(CellValue::string("shared")),
            ]],
            dependencies: Vec::new(),
        };
        let bytes = encode_table_asset(&input).expect("encode");
        let section_file = ParsedSectionFile::parse(&bytes).expect("parse sections");
        assert!(
            section_file
                .section_offset(TableSectionId::StringPool as u32)
                .is_some()
        );
        let pool = section_file
            .section_payload(&bytes, TableSectionId::StringPool as u32)
            .expect("string pool section");
        assert_eq!(pool.len(), 4 + "shared".len());
        let view = crate::table::view::TableView::from_bytes(&bytes).expect("view");
        assert_eq!(view.cell_as_str(0, 10), Some("shared"));
        assert_eq!(view.cell_as_str(1, 10), Some("shared"));
        assert_eq!(view.row_name(0), Some("shared"));
    }

    #[test]
    fn dependency_index_roundtrips() {
        let dependency = TableDependency {
            column_crc: 30,
            target_table_name_crc: 0xaaa_bbbb,
            target_schema_hash: SchemaHash(0x0123_4567_89ab_cdef),
            kind: DEPENDENCY_KIND_FOREIGN_KEY,
        };
        let input = EncodeInput {
            table_name: Cow::Borrowed(TEST_TABLE_NAME),
            schema_hash: SchemaHash(1),
            table_name_crc: test_table_crc(),
            row_type_crc: 0x3333_4444,
            columns: vec![ColumnDescriptor {
                crc: 30,
                cell_type: CellType::Scalar(ScalarType::ForeignKey),
                flags: 0,
            }],
            rows: vec![EncodeRow {
                key_crc: 100,
                debug_name: None,
                row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 100),
            }],
            cells: vec![vec![Some(CellValue::foreign_key(ForeignKeyValue::row(
                RowIndex::from_one_based(1).expect("row index"),
            )))]],
            dependencies: vec![dependency],
        };
        let bytes = encode_table_asset(&input).expect("encode");
        let section_file = ParsedSectionFile::parse(&bytes).expect("parse sections");
        assert!(
            section_file
                .section_offset(TableSectionId::DependencyIndex as u32)
                .is_some()
        );
        let body = decode_body(&bytes).expect("decode");
        assert_eq!(body.dependencies, vec![dependency]);
    }
}
