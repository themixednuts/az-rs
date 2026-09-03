//! `GameData` table asset format constants.
//!
//! On-disk table assets use the `AZTBL\0\0\0` magic tag plus the separate
//! header version field. Authoring files are normalized before this format is
//! emitted.

use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};

use az_asset_builder::{ProductFormat, ProductFormatId, ProductFormatRegistration};
use az_core::{AssetData, AssetType, AssetTypeRegistration, AzRtti, AzTypeInfo};
use uuid::{Uuid, uuid};

pub struct GameDataTableAssetData;

impl AzTypeInfo for GameDataTableAssetData {
    const NAME: &'static str = "AzGameData::GameDataTableAssetData";
    const TYPE_ID: Uuid = uuid!("024aa725-8f21-4fd5-b672-9ac0f4a11f23");
}

impl AzRtti for GameDataTableAssetData {}

impl AssetData for GameDataTableAssetData {
    const STABLE_NAME: &'static str = "azoth.gamedata.table";
}

#[derive(ProductFormat)]
#[product_format(id = "azoth.gamedata.table", version = 2, asset = GameDataTableAssetData)]
pub struct GameDataTableProductFormat;

/// `AssetCatalog` type for built `GameData` table assets.
pub const GAMEDATA_TABLE_ASSET_TYPE: AssetType = GameDataTableAssetData::ASSET_TYPE;

/// On-disk magic tag for `GameData` table assets (`AZTBL\0\0\0`).
pub const GAMEDATA_TABLE_MAGIC: [u8; 8] = *b"AZTBL\0\0\0";
/// Sectioned `AzSectionFile` container with cold GUID/name sections and hot column payloads.
pub const GAMEDATA_TABLE_VERSION: u32 = <GameDataTableProductFormat as ProductFormat>::VERSION;
/// Asset-builder byte-format id for `GameData` table assets.
pub const GAMEDATA_TABLE_FORMAT_ID: ProductFormatId =
    <GameDataTableProductFormat as ProductFormat>::ID;

/// The `GameData` table asset type.
#[must_use]
pub const fn asset_types() -> [AssetTypeRegistration; 1] {
    [AssetTypeRegistration::for_asset::<GameDataTableAssetData>().with_owner("gamedata::format")]
}

/// The `GameData` table byte contract.
#[must_use]
pub const fn product_formats() -> [ProductFormatRegistration; 1] {
    [ProductFormatRegistration::for_format::<
        GameDataTableProductFormat,
    >()]
}

/// Catalog-relative directory `GameData` table products live under.
///
/// The whole product route is `tables/<family>/<table>.aztbl`, where the family
/// segment is [`family_directory`] of the table's row schema.
pub const TABLE_PRODUCT_ROOT: &str = "tables";

/// The product directory every table sharing one merged row schema builds into.
///
/// Plain `snake_case` of the schema name. This is the *only* implementation of
/// the folder-to-family contract: the runtime derives a family directory from
/// `R::SCHEMA` through it, and a project builder routes its products through
/// the same call, so the two can never disagree about where a table lives.
///
/// Directories are not Rust modules, so a schema named `Match` yields `match`
/// and never `match_`. Codegen's Rust-module naming is a separate job under a
/// separate name; the two shared one once and drifted.
///
/// A leading digit is prefixed with `_` because a path segment starting with a
/// digit is legal but reads as a numbered sibling, and `(s)` / `(S)` in a
/// datasheet name folds into a plain plural before the case conversion.
#[must_use]
pub fn family_directory(schema_name: &str) -> String {
    let schema_name = schema_name.replace("(s)", "s").replace("(S)", "S");
    let mut directory = schema_name.to_snake_case();
    directory.retain(|character| character.is_ascii_alphanumeric() || character == '_');
    if directory.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        directory.insert(0, '_');
    }
    directory
}

/// Sections inside one built `GameData` table asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum TableSectionId {
    Schema = 1,
    HotColumns = 2,
    RowGuids = 3,
    StringPool = 4,
    RowKeyAliases = 5,
    DebugNames = 6,
    DependencyIndex = 7,
    TableIdentity = 8,
}

#[cfg(test)]
mod tests {
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };

    use super::*;

    declare_caps!(FormatCaps:);

    const FORMAT: ContributionDescriptor = ContributionDescriptor {
        gem: GemId::new("azoth.gamedata"),
        contribution: ContributionId::new("format"),
        roles: &[],
    };

    struct Format;

    impl Contribution for Format {
        type Caps = FormatCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            FORMAT
        }

        fn register(&self, ctx: &mut GemContext<'_, FormatCaps>) {
            ctx.registrar::<AssetTypeRegistration>()
                .register_many(asset_types());
            ctx.registrar::<ProductFormatRegistration>()
                .register_many(product_formats());
        }
    }

    #[test]
    fn the_composed_table_format_carries_its_byte_contract() {
        let mut composer = Composer::new(GemTargetRole::AssetWorker);
        composer
            .add(Format, ProductActivation::default())
            .expect("an empty floor composes");

        let registration = az_asset_builder::composed_product_format(
            composer.registries(),
            GAMEDATA_TABLE_FORMAT_ID,
        )
        .expect("the gamedata table format is composed");
        assert_eq!(registration.entry.id(), GAMEDATA_TABLE_FORMAT_ID);
        assert_eq!(registration.entry.current_version(), GAMEDATA_TABLE_VERSION);

        let report = composer.finalize().expect("composition is valid");
        assert!(
            report
                .entries
                .iter()
                .filter(|entry| entry.registry == "product-format")
                .all(|entry| entry.instance.gem.as_str() == "azoth.gamedata")
        );
    }
}
