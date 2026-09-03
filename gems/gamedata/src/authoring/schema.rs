use az_asset_builder::{
    AssetBuilderPattern, SourceFormat, SourceSchemaRegistration, SourceSchemaType,
};

#[derive(SourceFormat)]
#[source(schema = "azoth.gamedata.TableSource", pattern = "gamedata/*.ron")]
pub struct GameDataTableSourceFormat;

/// Generic source schema for editor-created `GameData` table source files.
pub const GAMEDATA_TABLE_SOURCE_SCHEMA: SourceSchemaType =
    match <GameDataTableSourceFormat as SourceFormat>::SCHEMA {
        Some(schema) => schema,
        None => panic!("GameDataTableSourceFormat declares a schema"),
    };

/// Static slice form used by project-owned table builders.
pub const GAMEDATA_TABLE_SOURCE_SCHEMA_TYPES: &[SourceSchemaType] = &[GAMEDATA_TABLE_SOURCE_SCHEMA];

/// The `GameData` table authoring source schema.
#[must_use]
pub const fn source_schemas() -> [SourceSchemaRegistration; 1] {
    [
        SourceSchemaRegistration::for_source::<GameDataTableSourceFormat>()
            .with_label("GameData Table")
            .with_category("GameData")
            .with_creatable_file("gamedata", &["ron"]),
    ]
}

/// Path patterns for `GameData` table source files.
///
/// Builders should combine these patterns with
/// [`GAMEDATA_TABLE_SOURCE_SCHEMA_TYPES`] so broad `.ron` sources do not
/// collide with project document schemas.
#[must_use]
pub const fn table_source_patterns() -> &'static [AssetBuilderPattern] {
    <GameDataTableSourceFormat as SourceFormat>::PATTERNS
}
