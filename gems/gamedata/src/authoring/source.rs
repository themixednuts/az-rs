use std::borrow::Cow;

use ron::Value;
use serde::{Deserialize, Serialize};

use crate::GameDataError;
use crate::descriptor::AuthoredTableSchema;
use crate::table::encode::CellValue;
use crate::table_set::ColumnSchema;

use super::value::{field_cell_value, row_fields, row_identity};

/// Self-describing authored `GameData` table source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableSourceEnvelope {
    name: String,
    schema: String,
    key: Option<String>,
    rows: Vec<Value>,
}

impl TableSourceEnvelope {
    #[inline]
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        schema: impl Into<String>,
        key: Option<String>,
        rows: Vec<Value>,
    ) -> Self {
        Self {
            name: name.into(),
            schema: schema.into(),
            key,
            rows,
        }
    }

    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    #[must_use]
    pub fn schema(&self) -> &str {
        &self.schema
    }

    #[inline]
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }

    #[inline]
    #[must_use]
    pub fn rows(&self) -> &[Value] {
        &self.rows
    }

    #[inline]
    #[must_use]
    pub const fn rows_mut(&mut self) -> &mut Vec<Value> {
        &mut self.rows
    }
}

/// Decodes the only supported authored `GameData` RON shape.
///
/// # Errors
///
/// Returns [`GameDataError::Decode`] when the bytes are not UTF-8, when RON
/// parsing rejects the envelope, or when the parsed envelope leaves `name`,
/// `schema`, or a supplied `key` blank.
pub fn decode_table_source_ron(source_bytes: &[u8]) -> Result<TableSourceEnvelope, GameDataError> {
    let text = std::str::from_utf8(source_bytes)
        .map_err(|err| GameDataError::Decode(format!("table source is not UTF-8: {err}")))?;
    let source = ron::from_str::<TableSourceEnvelope>(text).map_err(|err| {
        GameDataError::Decode(format!("parse GameData table RON envelope: {err}"))
    })?;
    if source.name.trim().is_empty() {
        return Err(GameDataError::Decode(
            "GameData table RON envelope has an empty `name`".to_owned(),
        ));
    }
    if source.schema.trim().is_empty() {
        return Err(GameDataError::Decode(format!(
            "GameData table `{}` RON envelope has an empty `schema`",
            source.name
        )));
    }
    if source
        .key
        .as_deref()
        .is_some_and(|key| key.trim().is_empty())
    {
        return Err(GameDataError::Decode(format!(
            "GameData table `{}` RON envelope has an empty `key`",
            source.name
        )));
    }
    Ok(source)
}

#[derive(Debug, Clone)]
pub(super) enum ParsedCell {
    Value(CellValue<'static>),
    ForeignKey(String),
    ForeignKeys(Vec<String>),
}

#[derive(Debug)]
pub(super) struct ParsedRow {
    pub(super) key_crc: u32,
    pub(super) debug_name: Option<Cow<'static, str>>,
    pub(super) cells: Vec<Option<ParsedCell>>,
}

#[derive(Debug)]
pub(super) struct ParsedTable<'a> {
    pub(super) table: &'a AuthoredTableSchema<'a>,
    pub(super) fields: Vec<ColumnSchema>,
    pub(super) rows: Vec<ParsedRow>,
}

pub(super) fn parse_table_source<'a>(
    table: &'a AuthoredTableSchema<'a>,
    rows: &[Value],
) -> Result<ParsedTable<'a>, GameDataError> {
    let fields = table.columns().collect::<Vec<_>>();
    let mut parsed_rows = Vec::with_capacity(rows.len());

    for (row_index, row) in rows.iter().enumerate() {
        let row = row_fields(table, row_index, row)?;
        let mut row_cells = Vec::with_capacity(fields.len());
        for field in &fields {
            row_cells.push(field_cell_value(table, *field, row_index, &row)?);
        }
        let (key_crc, debug_name) = row_identity(table, row_index, &row_cells)?;
        parsed_rows.push(ParsedRow {
            key_crc,
            debug_name,
            cells: row_cells,
        });
    }

    Ok(ParsedTable {
        table,
        fields,
        rows: parsed_rows,
    })
}
