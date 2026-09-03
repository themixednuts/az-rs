//! `GameData` table asset header validation and decode.
//!
//! The sectioned `AzSectionFile` container keeps physical identity and cold
//! GUID/name sections separate from hot column payloads (ADR 0002).

use crate::GameDataError;
use crate::format::{GAMEDATA_TABLE_MAGIC, GAMEDATA_TABLE_VERSION};
use crate::release::SchemaHash;
use crate::table::TableDependency;
use crate::table::body::TableBody;
use crate::table::encode::{decode_body, decode_table_name, parse_header};
#[cfg(test)]
use crate::table::view::TableView;

const HEADER_LEN: usize = 16;

/// Parsed `GameData` table asset bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct TableAsset {
    bytes: Vec<u8>,
    header: TableHeader,
    table_name: Box<str>,
    body: Option<TableBody>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableHeader {
    pub flags: u32,
    pub schema_hash: SchemaHash,
    pub table_name_crc: u32,
    pub row_type_crc: u32,
    pub row_count: u32,
    pub column_count: u32,
}

impl TableAsset {
    /// Parses a compiled table asset and pins its physical name to the header.
    ///
    /// # Errors
    ///
    /// Returns any error [`parse_table_header`] returns — a short buffer, the
    /// wrong magic, or an unsupported version — plus
    /// [`GameDataError::Decode`] when the name section is missing or not UTF-8,
    /// or when the lowercase CRC of the decoded table name disagrees with the
    /// `table_name_crc` in the header.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, GameDataError> {
        let header = parse_table_header(&bytes)?;
        let table_name = decode_table_name(&bytes)?;
        let actual_table_name_crc = az_core::crc::Crc32::from_str_lower(&table_name).value();
        if actual_table_name_crc != header.table_name_crc {
            return Err(GameDataError::Decode(format!(
                "GameData physical table `{table_name}` has lowercase CRC {actual_table_name_crc:#010x}, header contains {:#010x}",
                header.table_name_crc,
            )));
        }
        Ok(Self {
            bytes,
            header,
            table_name,
            body: None,
        })
    }

    #[must_use]
    pub const fn header(&self) -> TableHeader {
        self.header
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn table_name(&self) -> &str {
        &self.table_name
    }

    pub(crate) fn body(&mut self) -> Result<&TableBody, GameDataError> {
        if self.body.is_none() {
            self.body = Some(decode_body(&self.bytes)?);
        }
        Ok(self.body.as_ref().expect("cached table body"))
    }

    /// Decodes the body on first use and returns its dependency table.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] when the body sections cannot be
    /// decoded: a truncated section table or cell payload, a VLQ that runs
    /// past `u64`, an unknown cell-type or section id, a section length that
    /// disagrees with the header's row or column count, or a string or list
    /// span that points outside the asset bytes.
    pub fn dependencies(&mut self) -> Result<&[TableDependency], GameDataError> {
        Ok(self.body()?.dependencies())
    }

    #[cfg(test)]
    pub(crate) fn view(&mut self) -> Result<TableView<'_>, GameDataError> {
        self.body()?;
        let Self { bytes, body, .. } = self;
        let body = body.as_ref().expect("cached table body");
        TableView::from_parts(bytes, body)
    }
}

/// Checks the fixed table-asset preamble: length, magic, and version.
///
/// # Errors
///
/// Returns [`GameDataError::Decode`] when `bytes` is shorter than the fixed
/// header, does not start with the `GameData` table magic, or declares a
/// version other than the one this build reads.
///
/// # Panics
///
/// Panics if the version field cannot be read as four bytes. The length check
/// above makes that unreachable.
pub fn validate_header(bytes: &[u8]) -> Result<(), GameDataError> {
    let header = bytes.get(..HEADER_LEN).ok_or_else(|| {
        GameDataError::Decode(format!(
            "GameData table header too short: {} bytes (need {HEADER_LEN})",
            bytes.len()
        ))
    })?;
    if header[..8] != GAMEDATA_TABLE_MAGIC {
        return Err(GameDataError::Decode(format!(
            "expected GameData table magic, got {:?}",
            &header[..8]
        )));
    }
    let version = u32::from_le_bytes(header[8..12].try_into().expect("slice length"));
    if version != GAMEDATA_TABLE_VERSION {
        return Err(GameDataError::Decode(format!(
            "unsupported GameData table version {version} (expected {GAMEDATA_TABLE_VERSION})"
        )));
    }
    Ok(())
}

/// Reads the compiled table header after checking the preamble.
///
/// # Errors
///
/// Returns any error [`validate_header`] returns, plus
/// [`GameDataError::Decode`] when the SCHEMA or HOT section is absent or too
/// short to hold the schema hash, name and row-type CRCs, and the row and
/// column counts.
pub fn parse_table_header(bytes: &[u8]) -> Result<TableHeader, GameDataError> {
    validate_header(bytes)?;
    parse_header(bytes)
}

#[must_use]
pub fn is_table_asset(bytes: &[u8]) -> bool {
    validate_header(bytes).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::body::{CellType, CellValue, ColumnDescriptor, ScalarType};
    use crate::table::encode::{
        EncodeInput, EncodeRow, encode_table_asset, fixture_table_asset_bytes, import_row_guid,
    };

    #[test]
    fn accepts_valid_header() {
        let bytes = fixture_table_asset_bytes(SchemaHash(1), "TestTable", 0x3333_4444, 1);
        validate_header(&bytes).expect("valid header");
        assert!(is_table_asset(&bytes));
    }

    #[test]
    fn rejects_wrong_magic() {
        let bytes = b"NOTATBL01\0\x01\0\0\0\0\0\0\0\0";
        assert!(!is_table_asset(bytes));
    }

    #[test]
    fn parse_table_header_reads_schema_and_hot_sections() {
        let bytes = fixture_table_asset_bytes(
            SchemaHash(0x0123_4567_89ab_cdef),
            "TestTable",
            0x3333_4444,
            42,
        );
        let parsed = parse_table_header(&bytes).expect("parse header");
        assert_eq!(parsed.schema_hash.0, 0x0123_4567_89ab_cdef);
        assert_eq!(
            parsed.table_name_crc,
            az_core::crc::Crc32::from_str_lower("TestTable").value()
        );
        assert_eq!(parsed.row_type_crc, 0x3333_4444);
        assert_eq!(parsed.row_count, 42);
        assert_eq!(parsed.column_count, 0);
    }

    #[test]
    fn table_asset_from_bytes_retains_header() {
        let bytes = fixture_table_asset_bytes(
            SchemaHash(0x0123_4567_89ab_cdef),
            "TestTable",
            0x3333_4444,
            3,
        );
        let asset = TableAsset::from_bytes(bytes.clone()).expect("parse table asset");
        assert_eq!(asset.bytes(), bytes);
        assert_eq!(asset.table_name(), "TestTable");
        assert_eq!(asset.header().row_count, 3);
    }

    #[test]
    fn table_asset_decodes_hot_columns_without_string_allocs() {
        let input = EncodeInput {
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
                    debug_name: None,
                    row_guid: import_row_guid(0x1111_2222, 0x3333_4444, 100),
                },
                EncodeRow {
                    key_crc: 200,
                    debug_name: None,
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
        let bytes = encode_table_asset(&input).expect("encode");
        let mut asset = TableAsset::from_bytes(bytes).expect("parse table asset");
        let view = asset.view().expect("decode body");
        assert_eq!(view.row_key_crc(0), Some(100));
        assert_eq!(view.cell_as_str(0, 10), Some("alpha"));
        assert_eq!(view.cell_as_f64(0, 20), Some(1.5));
    }
}
