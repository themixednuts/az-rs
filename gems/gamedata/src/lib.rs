//! Engine-wide `GameData` runtime contracts.
//!
//! Release identity, table assets, merged row schemas, and typed table access
//! live here. Game-specific indexes and semantic managers belong in a title
//! project crate.

#![forbid(unsafe_code)]

extern crate self as gamedata;

#[cfg(feature = "authoring")]
pub mod authoring;
#[cfg(feature = "bevy")]
pub mod bevy;
#[cfg(feature = "authoring")]
pub mod dependencies;
pub mod descriptor;
mod error;
pub mod format;
pub mod game_system;
pub mod identity;
pub mod manager;
pub mod release;
pub mod release_validation;
pub mod row;
pub mod table;
pub mod table_set;

use az_gem_contract::{Contribution, GemContext, contribution};

pub use az_asset::ASSET_CATALOG_FILE_NAME as ASSET_CATALOG_PATH;
#[cfg(feature = "authoring")]
pub use dependencies::{
    collect_table_dependencies, collect_table_dependencies_with_schema, dependency_edge_set,
    verify_dependency_parity,
};
pub use descriptor::{
    AuthoredTableSchema, AuthoredTableSchemaError, ColumnSchemaDescriptor, EnumVariantDescriptor,
    ForeignKeyTargetDescriptor, RowSchema, RowSchemaCatalog, RowSchemaDescriptor,
    RowSchemaDescriptorError, row_schema_hash,
};
pub use error::GameDataError;
pub use format::{
    GAMEDATA_TABLE_ASSET_TYPE, GAMEDATA_TABLE_FORMAT_ID, GAMEDATA_TABLE_VERSION, TableSectionId,
};
pub use game_system::{SchemaRow, SchemaRowRef, SchemaTable, SchemaValue, TableId};
pub use identity::RowIndex;
pub use release::{
    GameDataDependency, GameDataDependencyKind, GameDataRelease, GameDataReleaseId, ProjectionHash,
    SchemaHash,
};
pub use release_validation::{
    validate_release_against_world_node, validate_release_pins_non_empty,
};
pub use row::{GameDataKeyValue, GameDataRow, GameDataSchemaRow};
pub use table::{
    AtomType, CellRef, CellType, DEPENDENCY_KIND_FOREIGN_KEY, ListElementType, PairType, PairValue,
    RangeBounds, RangeEndpointType, RangeType, Row, ScalarType, TableAsset, TableDependency,
    TableHeader, TextRef, is_table_asset, parse_table_header, validate_header,
};
pub use table_set::{Atom, Cell, EnumRepresentation, EnumVariantMeta, ForeignKeyMeta, TableEnum};

#[cfg(feature = "derive")]
pub use gamedata_derive::GameDataRow;

// This package implements both of its gem's contributions, so each block names
// which one it is with a bare token. Both are declared unconditionally: the
// `authoring` cargo feature decides what the crate *links*, not whether a
// declared contribution exists, so the feature belongs inside a body and never
// in whether the entry item is there to call.

/// The gem's `authoring` contribution: the project-host half.
///
/// It registers nothing, and that is the honest shape rather than an oversight.
/// What `project-host` reads from this gem is the manager catalog it builds out
/// of composed [`RowSchema`] and [`manager::GameDataManagerShape`] entries —
/// and this gem *defines* those registry entry types without owning a single
/// entry of either. Every row schema and manager shape in a running host is a
/// project's generated catalog, contributed through
/// exactly these registrars), so a claim here would be a claim on somebody
/// else's content.
///
/// What the stanza carries instead is linkage: `project-host` names this
/// package with `features = ["authoring"]` because `gamedata::authoring` is
/// what its catalog code is written against. The compose-seam test holds the
/// registration surface to empty so a registration added later has to be
/// declared, not discovered.
///
/// Sealing is privacy: the generated `authoring_contribution` is the only way
/// in.
struct Authoring;

#[contribution(authoring)]
impl Contribution for Authoring {
    fn register(&self, _ctx: &mut GemContext<'_, Self::Caps>) {}
}

/// The gem's `builders` contribution: the whole builder catalog it owns.
///
/// All three families land here because all three are read by the two roles
/// this stanza names and by nothing else. `asset-processor` and `asset-worker`
/// are where a builder catalog is assembled, validated and served: the source
/// schema is the classifier that makes a `gamedata/*.ron` a table source and
/// the descriptor those hosts hand the editor, the product format is what the
/// worker looks up before it writes AZTBL bytes, and the asset type is what
/// names the built product in a catalog and in an analysis fingerprint. A
/// source root the host cannot classify is a source root nothing builds, so
/// splitting one of these onto the `project-host` stanza would leave the
/// pipeline unable to see its own format.
///
/// Sealing is privacy: the generated `builders_contribution` is the only way
/// in.
struct Builders;

#[contribution(builders)]
impl Contribution for Builders {
    fn register(&self, ctx: &mut GemContext<'_, Self::Caps>) {
        ctx.registrar::<az_core::AssetTypeRegistration>()
            .register_many(format::asset_types());
        ctx.registrar::<az_asset_builder::ProductFormatRegistration>()
            .register_many(format::product_formats());
        #[cfg(feature = "authoring")]
        ctx.registrar::<az_asset_builder::SourceSchemaRegistration>()
            .register_many(authoring::source_schemas());
    }
}
