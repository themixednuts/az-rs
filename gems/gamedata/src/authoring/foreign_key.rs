use std::collections::BTreeMap;

use crate::GameDataError;
use crate::descriptor::AuthoringTableSchema as TableSchema;
use crate::identity::RowIndex;
use crate::table::encode::{
    CellValue, DEPENDENCY_KIND_FOREIGN_KEY, ForeignKeyValue, ListValue, TableDependency,
};
use crate::table::{CellType, ListElementType, ScalarType};
use crate::table_set::{ColumnSchema, ForeignKeyMeta};

use super::source::{ParsedCell, ParsedTable};

#[derive(Debug)]
pub(super) struct SourceIndex<'a> {
    tables: BTreeMap<(u32, u32), IndexedSourceTable<'a>>,
}

#[derive(Debug)]
struct IndexedSourceTable<'a> {
    table: &'a dyn TableSchema,
    row_key: BTreeMap<String, RowIndex>,
}

impl<'a> SourceIndex<'a> {
    pub(super) fn new(sources: Vec<ParsedTable<'a>>) -> Result<Self, GameDataError> {
        let mut tables = BTreeMap::new();
        for source in sources {
            let key = (source.table.crc(), source.table.row_crc());
            if tables.contains_key(&key) {
                return Err(GameDataError::Decode(format!(
                    "duplicate table source for `{}` ({:#010x}/{:#010x})",
                    source.table.name(),
                    source.table.crc(),
                    source.table.row_crc()
                )));
            }
            let mut row_key = BTreeMap::new();
            for (row_index, row) in source.rows.iter().enumerate() {
                let Some(name) = row.debug_name.as_deref().filter(|name| !name.is_empty()) else {
                    continue;
                };
                let Some(index) =
                    RowIndex::from_one_based(u32::try_from(row_index + 1).map_err(|_| {
                        GameDataError::Decode(format!(
                            "table `{}` row index exceeds u32",
                            source.table.name()
                        ))
                    })?)
                else {
                    continue;
                };
                let canonical_name = canonical_row_key(name);
                // Native caches preserve every row while keyed lookups resolve
                // duplicate keys to the first authored row. FK lowering must
                // select the same row as the runtime index.
                row_key.entry(canonical_name).or_insert(index);
            }
            tables.insert(
                key,
                IndexedSourceTable {
                    table: source.table,
                    row_key,
                },
            );
        }
        Ok(Self { tables })
    }

    fn target(&self, fk: ForeignKeyMeta) -> Result<&IndexedSourceTable<'a>, GameDataError> {
        self.tables
            .get(&(fk.target_table_crc(), fk.target_row_crc()))
            .ok_or_else(|| {
                GameDataError::Decode(format!(
                    "missing table source for generated FK target `{}.{}`",
                    fk.target_table(),
                    fk.target_column()
                ))
            })
    }

    fn row_index(
        &self,
        field: ColumnSchema,
        row_index: usize,
        name: &str,
    ) -> Result<RowIndex, GameDataError> {
        let fk = exact_foreign_key(field)?;
        let target = self.target(fk)?;
        target
            .row_key
            .get(&canonical_row_key(name))
            .copied()
            .ok_or_else(|| {
                GameDataError::Decode(format!(
                    "row {row_index} field `{}` references missing generated FK `{}` in `{}.{}`",
                    field.name(),
                    name,
                    fk.target_table(),
                    fk.target_column()
                ))
            })
    }
}

fn canonical_row_key(name: &str) -> String {
    name.trim()
        .trim_start_matches(['!', '+'])
        .trim()
        .to_ascii_lowercase()
}

pub(super) fn resolve_cell(
    table: &dyn TableSchema,
    field: ColumnSchema,
    row_index: usize,
    cell: Option<ParsedCell>,
    index: &SourceIndex<'_>,
) -> Result<Option<CellValue<'static>>, GameDataError> {
    let Some(cell) = cell else {
        return Ok(None);
    };
    match (field.cell_type(), cell) {
        (CellType::Scalar(ScalarType::ForeignKey), ParsedCell::ForeignKey(name)) => {
            if name.is_empty() {
                return Ok(None);
            }
            let row = index.row_index(field, row_index, &name)?;
            Ok(Some(CellValue::foreign_key(ForeignKeyValue::row(row))))
        }
        (
            CellType::List(ListElementType::Scalar(ScalarType::ForeignKey)),
            ParsedCell::ForeignKeys(names),
        ) => {
            let mut rows = Vec::with_capacity(names.len());
            for name in names {
                if name.is_empty() {
                    continue;
                }
                rows.push(ForeignKeyValue::row(
                    index.row_index(field, row_index, &name)?,
                ));
            }
            Ok(Some(CellValue::List(ListValue::foreign_keys(rows))))
        }
        (_, ParsedCell::Value(value)) => Ok(Some(value)),
        (
            CellType::Scalar(ScalarType::ForeignKey)
            | CellType::List(ListElementType::Scalar(ScalarType::ForeignKey)),
            other,
        ) => Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` has unresolved source value {other:?}",
            table.name(),
            field.name()
        ))),
        (cell_type, other) => Err(GameDataError::Decode(format!(
            "table `{}` row {row_index} field `{}` cannot encode unresolved source value {other:?} as {cell_type:?}",
            table.name(),
            field.name()
        ))),
    }
}

fn exact_foreign_key(field: ColumnSchema) -> Result<ForeignKeyMeta, GameDataError> {
    match field.foreign_keys() {
        [fk] => Ok(*fk),
        [] => Err(GameDataError::Decode(format!(
            "field `{}` is encoded as a foreign key without a generated target",
            field.name()
        ))),
        fks => Err(GameDataError::Decode(format!(
            "field `{}` has {} generated FK targets; ambiguous FKs must stay string/list typed",
            field.name(),
            fks.len()
        ))),
    }
}

pub(super) fn table_dependencies(
    table: &dyn TableSchema,
    fields: &[ColumnSchema],
    index: &SourceIndex<'_>,
) -> Result<Vec<TableDependency>, GameDataError> {
    let mut dependencies = Vec::new();
    for field in fields {
        for fk in field.foreign_keys() {
            let target = index.target(*fk)?;
            dependencies.push(TableDependency {
                column_crc: field.column_crc(),
                target_table_name_crc: fk.target_table_crc(),
                target_schema_hash: target.table.schema_hash(),
                kind: DEPENDENCY_KIND_FOREIGN_KEY,
            });
        }
    }
    dependencies.sort_by_key(|edge| (edge.column_crc, edge.target_table_name_crc));
    dependencies.dedup_by_key(|edge| (edge.column_crc, edge.target_table_name_crc));
    if dependencies.is_empty()
        && fields
            .iter()
            .any(|field| matches!(field.cell_type(), CellType::Scalar(ScalarType::ForeignKey)))
    {
        return Err(GameDataError::Decode(format!(
            "table `{}` projects foreign-key columns without generated targets",
            table.name()
        )));
    }
    Ok(dependencies)
}
