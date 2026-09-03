//! Editor and build workflow for authored `GameData` tables.
//!
//! Every source is a self-describing RON envelope. Projects generate one Rust
//! descriptor per merged logical row schema; physical tables are discovered
//! from source assets and are never represented by generated Rust types.
//!
//! What lives here is the machinery only an authoring host runs: the RON
//! compiler, the envelope reader, and the catalog/graph/fingerprint algorithms
//! over manager descriptors. The descriptors themselves are composition
//! contracts and live unconditionally in [`crate::manager`].

use std::borrow::Cow;

use crate::GameDataError;
use crate::descriptor::{AuthoredTableSchema, RowSchemaCatalog, RowSchemaDescriptorError};
use crate::table::CellType;
use crate::table_set::ColumnSchema;

mod foreign_key;
mod header;
mod list;
mod manager_catalog;
mod manager_fingerprint;
mod manager_graph;
mod number;
mod schema;
mod semantic;
mod source;
mod value;

pub use crate::table::encode::{
    CellValue, ColumnDescriptor, EncodeInput, EncodeRow, encode_table_asset, import_row_guid,
    import_row_guid_with_name,
};
pub use header::{Header, HeaderError, header};
pub use manager_catalog::{
    ManagerCatalogDiagnostic, ManagerCatalogEntry, ManagerCatalogInput, ManagerDeleteImpact,
    ResolvedManagerCatalog, build_manager_catalog,
};
pub use manager_fingerprint::{
    ManagerProjectionDependency, ManagerProjectionSource, manager_projection_fingerprint,
    manager_projection_fingerprint_with_deps,
};
pub use manager_graph::{
    ManagerGraphError, ManagerNode, ManagerNodeId, ResolvedManagerGraph, ResolvedManagerNode,
    build_manager_graph,
};
pub use schema::{
    GAMEDATA_TABLE_SOURCE_SCHEMA, GAMEDATA_TABLE_SOURCE_SCHEMA_TYPES, source_schemas,
    table_source_patterns,
};
pub use semantic::{FilterSet, FilterTerm, MatchFilter, ValueOp, ValueOps};
pub use source::{TableSourceEnvelope, decode_table_source_ron};

use foreign_key::{SourceIndex, resolve_cell, table_dependencies};
use source::{ParsedTable, parse_table_source};

/// Compiles one self-describing authored RON table into `AZTBL` bytes.
///
/// `sources` contains the other self-describing tables needed to resolve exact
/// foreign keys. No source path or generated physical-table descriptor is
/// consulted.
///
/// # Errors
///
/// Returns [`GameDataError::Decode`] when the row-schema catalog is invalid,
/// when `source_bytes` or any entry of `sources` is not a decodable authored
/// envelope, when a source names a row schema the catalog does not hold or a
/// key column that schema cannot key on, when a row fails typing against its
/// schema, or when a foreign key resolves to no row in the supplied sources.
/// Encoding itself adds the size limits of the table format: more columns,
/// rows, strings, or list elements than the section headers can address.
pub fn encode_table_source_ron(
    source_bytes: &[u8],
    schemas: RowSchemaCatalog,
    sources: &[&[u8]],
) -> Result<Vec<u8>, GameDataError> {
    validate_catalog(schemas)?;
    let current_source = decode_table_source_ron(source_bytes)?;
    let dependency_sources = sources
        .iter()
        .map(|bytes| decode_table_source_ron(bytes))
        .collect::<Result<Vec<_>, _>>()?;

    let current_schema = bind_source_schema(schemas, &current_source)?;
    let dependency_schemas = dependency_sources
        .iter()
        .map(|source| bind_source_schema(schemas, source))
        .collect::<Result<Vec<_>, _>>()?;

    let current = parse_table_source(&current_schema, current_source.rows())?;
    let mut indexed_sources = Vec::with_capacity(dependency_sources.len() + 1);
    indexed_sources.push(parse_table_source(&current_schema, current_source.rows())?);
    for (source, schema) in dependency_sources.iter().zip(&dependency_schemas) {
        if schema.crc() == current_schema.crc() && schema.row_crc() == current_schema.row_crc() {
            continue;
        }
        indexed_sources.push(parse_table_source(schema, source.rows())?);
    }

    let index = SourceIndex::new(indexed_sources)?;
    encode_parsed_table(current, &index)
}

/// Validates one self-describing authored RON table with the production
/// parser, row typing, and foreign-key resolver.
///
/// # Errors
///
/// Returns any error [`encode_table_source_ron`] returns; the encoded bytes
/// are discarded.
pub fn validate_table_source_ron(
    source_bytes: &[u8],
    schemas: RowSchemaCatalog,
    sources: &[&[u8]],
) -> Result<(), GameDataError> {
    encode_table_source_ron(source_bytes, schemas, sources).map(|_| ())
}

fn bind_source_schema(
    schemas: RowSchemaCatalog,
    source: &TableSourceEnvelope,
) -> Result<AuthoredTableSchema<'_>, GameDataError> {
    let row_schema = schemas.by_name(source.schema()).ok_or_else(|| {
        GameDataError::Decode(format!(
            "GameData table `{}` names unknown row schema `{}`",
            source.name(),
            source.schema()
        ))
    })?;
    AuthoredTableSchema::new(source.name(), row_schema, source.key())
        .map_err(|err| GameDataError::Decode(err.to_string()))
}

fn validate_catalog(schemas: RowSchemaCatalog) -> Result<(), GameDataError> {
    schemas.validate().map_err(|err| descriptor_error(&err))
}

fn descriptor_error(err: &RowSchemaDescriptorError) -> GameDataError {
    GameDataError::Decode(format!("invalid GameData row schema catalog: {err}"))
}

fn encode_parsed_table(
    parsed: ParsedTable<'_>,
    index: &SourceIndex<'_>,
) -> Result<Vec<u8>, GameDataError> {
    let mut encode_rows = Vec::with_capacity(parsed.rows.len());
    let mut cells = vec![Vec::with_capacity(parsed.rows.len()); parsed.fields.len()];

    for (row_index, row) in parsed.rows.into_iter().enumerate() {
        let row_guid = row.debug_name.as_deref().map_or_else(
            || import_row_guid(parsed.table.crc(), parsed.table.row_crc(), row.key_crc),
            |name| import_row_guid_with_name(parsed.table.crc(), parsed.table.row_crc(), name),
        );
        encode_rows.push(EncodeRow {
            key_crc: row.key_crc,
            debug_name: row.debug_name,
            row_guid,
        });

        for (column_index, cell) in row.cells.into_iter().enumerate() {
            let field = parsed.fields[column_index];
            cells[column_index].push(resolve_cell(parsed.table, field, row_index, cell, index)?);
        }
    }

    encode_table_asset(&EncodeInput {
        table_name: Cow::Borrowed(parsed.table.name()),
        schema_hash: parsed.table.schema_hash(),
        table_name_crc: parsed.table.crc(),
        row_type_crc: parsed.table.row_crc(),
        columns: parsed
            .fields
            .iter()
            .map(|field| ColumnDescriptor {
                crc: field.column_crc(),
                cell_type: field.cell_type(),
                flags: column_flags(*field),
            })
            .collect(),
        rows: encode_rows,
        cells,
        dependencies: table_dependencies(parsed.table, &parsed.fields, index)?,
    })
}

const fn column_flags(field: ColumnSchema) -> u32 {
    let mut flags = 0u32;
    if field.is_row_key() {
        flags |= 1;
    }
    if field.is_required() {
        flags |= 1 << 1;
    }
    if matches!(field.cell_type(), CellType::List(_)) {
        flags |= 1 << 2;
    }
    if !field.foreign_keys().is_empty() {
        flags |= 1 << 3;
    }
    flags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::table::view::TableView;
    use crate::table::{CellRef, ScalarType};
    use crate::{ColumnSchemaDescriptor, RowIndex, RowSchemaDescriptor};
    use az_core::crc::Crc32;

    const ITEM_COLUMNS: &[ColumnSchemaDescriptor] = &[
        ColumnSchemaDescriptor::new("item_id", "ItemID", CellType::Scalar(ScalarType::RowKey))
            .key_candidate(true),
        ColumnSchemaDescriptor::new("rarity", "Rarity", CellType::Scalar(ScalarType::U8))
            .with_enum_variants(&[
                crate::EnumVariantDescriptor::new("Common", &["common"], 1),
                crate::EnumVariantDescriptor::new("Rare", &["rare"], 2),
            ]),
    ];
    const LOADOUT_COLUMNS: &[ColumnSchemaDescriptor] = &[
        ColumnSchemaDescriptor::new(
            "loadout_id",
            "LoadoutID",
            CellType::Scalar(ScalarType::RowKey),
        )
        .key_candidate(true),
        ColumnSchemaDescriptor::new(
            "equipped_item",
            "EquippedItem",
            CellType::Scalar(ScalarType::ForeignKey),
        )
        .with_foreign_key_targets(&[crate::ForeignKeyMeta::new(
            "MasterItemDefinitions",
            "ItemData",
            "ItemID",
        )]),
    ];
    const OPEN_ENUM_COLUMNS: &[ColumnSchemaDescriptor] = &[
        ColumnSchemaDescriptor::new("entry_id", "EntryID", CellType::Scalar(ScalarType::RowKey))
            .key_candidate(true),
        ColumnSchemaDescriptor::new("kind", "Kind", CellType::Scalar(ScalarType::String))
            .with_enum_variants(&[crate::EnumVariantDescriptor::new("Known", &["known"], 1)]),
    ];
    const SCHEMAS: RowSchemaCatalog = RowSchemaCatalog::new(&[
        RowSchemaDescriptor::new("ItemData", ITEM_COLUMNS),
        RowSchemaDescriptor::new("LoadoutData", LOADOUT_COLUMNS),
        RowSchemaDescriptor::new("OpenEnumData", OPEN_ENUM_COLUMNS),
    ]);
    const ITEM_SOURCE: &[u8] = br#"(
        name: "MasterItemDefinitions",
        schema: "ItemData",
        key: Some("item_id"),
        rows: [
            (item_id: "Sword", rarity: "rare"),
            (item_id: "Axe", rarity: "common"),
        ],
    )"#;

    const DUPLICATE_ITEM_SOURCE: &[u8] = br#"(
        name: "MasterItemDefinitions",
        schema: "ItemData",
        key: Some("item_id"),
        rows: [
            (item_id: "Sword", rarity: "rare"),
            (item_id: "Axe", rarity: "common"),
            (item_id: "aXe", rarity: "rare"),
        ],
    )"#;

    const LOADOUT_SOURCE: &[u8] = br#"(
        name: "StartingLoadouts",
        schema: "LoadoutData",
        key: Some("loadout_id"),
        rows: [
            (loadout_id: "Start", equipped_item: "aXe"),
            (loadout_id: "Excluded", equipped_item: " !Sword "),
        ],
    )"#;

    const EVENT_ITEM_SOURCE: &[u8] = br#"(
        name: "EventItemDefinitions",
        schema: "ItemData",
        key: Some("item_id"),
        rows: [
            (item_id: "EventSword", rarity: "rare"),
        ],
    )"#;

    const STRICT_LOADOUT_SOURCE: &[u8] = br#"(
        name: "StrictStartingLoadouts",
        schema: "LoadoutData",
        key: Some("loadout_id"),
        rows: [
            (loadout_id: "Broken", equipped_item: "NotShippedYet"),
        ],
    )"#;

    const OPEN_ENUM_SOURCE: &[u8] = br#"(
        name: "OpenEnumValues",
        schema: "OpenEnumData",
        key: Some("entry_id"),
        rows: [
            (entry_id: "KnownEntry", kind: "known"),
            (entry_id: "FutureEntry", kind: "FutureValue"),
        ],
    )"#;

    #[test]
    fn table_source_schema_registers_creatable_file_workflow() {
        let [registration] = schema::source_schemas();
        assert_eq!(registration.schema_type(), GAMEDATA_TABLE_SOURCE_SCHEMA);
        let az_asset_builder::SourceSchemaAuthoring::File { workflow } = registration.authoring()
        else {
            panic!("GameData table source should be file-backed");
        };
        assert!(workflow.can_create());
        assert!(workflow.can_edit());
        assert_eq!(workflow.default_path_prefix(), "gamedata");
        assert_eq!(workflow.extensions(), &["ron"]);
        assert!(
            registration
                .source_patterns()
                .iter()
                .any(|pattern| pattern.matches("gamedata/items/item_definitions.ron"))
        );
        assert!(
            !registration
                .source_patterns()
                .iter()
                .any(|pattern| pattern.matches("coatgen/chunk.material.ron"))
        );
    }

    #[test]
    fn envelope_compiles_rows_enums_and_foreign_keys() {
        let item_bytes =
            encode_table_source_ron(ITEM_SOURCE, SCHEMAS, &[]).expect("compile item source");
        let loadout_bytes = encode_table_source_ron(LOADOUT_SOURCE, SCHEMAS, &[ITEM_SOURCE])
            .expect("compile loadout source");

        let item_view = TableView::from_bytes(&item_bytes).expect("item view");
        let loadout_view = TableView::from_bytes(&loadout_bytes).expect("loadout view");
        assert_eq!(
            item_view.cell(0, ITEM_COLUMNS[1].source_column_crc()),
            Some(CellRef::U8(2))
        );
        assert_eq!(
            loadout_view.cell_as_row_index(0, LOADOUT_COLUMNS[1].source_column_crc()),
            RowIndex::from_one_based(2)
        );
        assert_eq!(
            loadout_view.cell_as_row_index(1, LOADOUT_COLUMNS[1].source_column_crc()),
            RowIndex::from_one_based(1)
        );

        let item_header = crate::parse_table_header(&item_bytes).expect("item header");
        let other_item_bytes = encode_table_source_ron(EVENT_ITEM_SOURCE, SCHEMAS, &[])
            .expect("compile second item table");
        let other_header = crate::parse_table_header(&other_item_bytes).expect("other header");
        assert_ne!(item_header.table_name_crc, other_header.table_name_crc);
        assert_eq!(item_header.row_type_crc, other_header.row_type_crc);
        assert_eq!(item_header.schema_hash, other_header.schema_hash);
    }

    #[test]
    fn duplicate_source_keys_preserve_rows_and_foreign_keys_resolve_first_wins() {
        let item_bytes = encode_table_source_ron(DUPLICATE_ITEM_SOURCE, SCHEMAS, &[])
            .expect("compile source-faithful duplicate item rows");
        let loadout_bytes =
            encode_table_source_ron(LOADOUT_SOURCE, SCHEMAS, &[DUPLICATE_ITEM_SOURCE])
                .expect("compile foreign key against duplicate target rows");

        let item_view = TableView::from_bytes(&item_bytes).expect("duplicate item view");
        let loadout_view = TableView::from_bytes(&loadout_bytes).expect("loadout view");

        assert_eq!(item_view.row_count(), 3);
        assert_eq!(
            item_view.row_index_by_key_crc(Crc32::from_str_lower("AXE").value()),
            None
        );
        assert_eq!(
            loadout_view.cell_as_row_index(0, LOADOUT_COLUMNS[1].source_column_crc()),
            RowIndex::from_one_based(2)
        );
    }

    #[test]
    fn strict_foreign_keys_reject_dangling_values() {
        let error = encode_table_source_ron(STRICT_LOADOUT_SOURCE, SCHEMAS, &[ITEM_SOURCE])
            .expect_err("strict generated foreign key must reject a dangling value");
        assert!(
            error
                .to_string()
                .contains("references missing generated FK `NotShippedYet`"),
            "{error}"
        );
    }

    #[test]
    fn top_level_row_lists_are_rejected() {
        let err = encode_table_source_ron(br#"[(item_id: "Sword", rarity: "rare")]"#, SCHEMAS, &[])
            .expect_err("legacy list-only source must fail");
        assert!(err.to_string().contains("RON envelope"));
    }

    #[test]
    fn string_backed_enum_metadata_preserves_open_source_values() {
        let bytes = encode_table_source_ron(OPEN_ENUM_SOURCE, SCHEMAS, &[])
            .expect("compile open string enum source");
        let view = TableView::from_bytes(&bytes).expect("open enum table view");

        assert_eq!(
            view.cell_as_str(0, OPEN_ENUM_COLUMNS[1].source_column_crc()),
            Some("known")
        );
        assert_eq!(
            view.cell_as_str(1, OPEN_ENUM_COLUMNS[1].source_column_crc()),
            Some("FutureValue")
        );
    }
}
