//! Compose-seam tests for the `GameData` registries and the gem's own entry
//! items.
//!
//! Row schemas and manager shapes are contributed through registrars and read
//! back off the composed host's owned registries — there is no process-global
//! `GameData` registration state to observe. The gem's two contributions are
//! pinned the same way: what each one puts in a host's registries, at the roles
//! its stanza names.

#![cfg(feature = "authoring")]

use az_asset_builder::{ProductFormatRegistration, SourceSchemaRegistration};
use az_core::AssetTypeRegistration;
use az_gem_contract::{
    ComposeError, Composer, Contribution, ContributionDescriptor, ContributionId, GemContext,
    GemId, GemTargetRole, RegistryEntry, declare_caps,
};
use azoth_gamedata_ids::{
    GEM,
    contributions::{AUTHORING, BUILDERS},
};
use gamedata::authoring::{
    ManagerCatalogInput, ManagerNodeId, ManagerProjectionSource, build_manager_catalog,
};
// The descriptors themselves are composition contracts and live unconditionally
// in `gamedata::manager`, not behind the `authoring` feature.
use gamedata::manager::{
    GameDataManagerInput, GameDataManagerShape, ManagerShapeKind, ProviderTarget,
    TableInputDescriptor,
};
use gamedata::release::SchemaHash;
use gamedata::{
    CellType, ColumnSchemaDescriptor, RowSchema, RowSchemaCatalog, RowSchemaDescriptor, ScalarType,
};
use gamedata::{authoring_contribution, builders_contribution};

const ITEM_COLUMNS: &[ColumnSchemaDescriptor] =
    &[
        ColumnSchemaDescriptor::new("item_id", "ItemID", CellType::Scalar(ScalarType::RowKey))
            .key_candidate(true),
    ];
const ITEM_SCHEMA: RowSchemaDescriptor = RowSchemaDescriptor::new("ItemData", ITEM_COLUMNS);
const CATALOG: RowSchemaCatalog = RowSchemaCatalog::new(&[ITEM_SCHEMA]);

const TABLE: TableInputDescriptor = TableInputDescriptor::new("RegistryTable", "RegistryRow");
const INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Table(TABLE)];
const MANAGER: GameDataManagerShape = GameDataManagerShape::new(
    "RegistryManager",
    ManagerShapeKind::SingleTableIndex,
    INPUTS,
);

/// The shape a project code generator emits into generated `rows/mod.rs`: one
/// `register` over any declared capability set, registering the whole catalog.
/// Compiling it here pins the emission contract.
fn register_rows<D>(ctx: &mut GemContext<'_, D>) {
    ctx.registrar::<RowSchema>().register_many(CATALOG.rows());
}

declare_caps!(GameDataCaps:);

struct Schema;

impl Contribution for Schema {
    type Caps = GameDataCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("sample.gamedata"),
            contribution: ContributionId::new("schema"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, GameDataCaps>) {
        register_rows(ctx);
        ctx.registrar::<GameDataManagerShape>().register(MANAGER);
    }
}

/// A second gem claiming the same manager name.
struct Shadow;

impl Contribution for Shadow {
    type Caps = GameDataCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("sample.shadow"),
            contribution: ContributionId::new("schema"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, GameDataCaps>) {
        ctx.registrar::<GameDataManagerShape>().register(MANAGER);
    }
}

/// A second gem claiming the same merged row schema.
struct Twin;

impl Contribution for Twin {
    type Caps = GameDataCaps;

    fn descriptor(&self) -> ContributionDescriptor {
        ContributionDescriptor {
            gem: GemId::new("sample.twin"),
            contribution: ContributionId::new("schema"),
            roles: &[],
        }
    }

    fn register(&self, ctx: &mut GemContext<'_, GameDataCaps>) {
        register_rows(ctx);
    }
}

fn compose(role: GemTargetRole) -> Composer {
    let mut composer = Composer::new(role);
    composer
        .add(Schema, az_gem_contract::ProductActivation::default())
        .expect("gamedata requires no capability");
    composer
}

#[test]
fn gamedata_composes_into_the_authoring_hosts() {
    for role in [GemTargetRole::ProjectHost, GemTargetRole::AssetWorker] {
        let report = compose(role).finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "gamedata is unconditional");
        assert!(
            report
                .entries
                .iter()
                .any(|entry| entry.registry == "gamedata-manager"
                    && entry.key == "RegistryManager"),
            "manager must compose into `{role}`"
        );
    }
}

#[test]
fn a_catalog_composes_one_entry_per_row_schema() {
    let composer = compose(GemTargetRole::ProjectHost);
    let rows = composer
        .registries()
        .get::<RowSchema>()
        .expect("row schemas were registered");

    assert_eq!(rows.len(), CATALOG.len());
    assert_eq!(
        rows.entries()
            .map(|row| row.descriptor())
            .collect::<Vec<_>>(),
        CATALOG.schemas().iter().collect::<Vec<_>>()
    );
    assert!(
        rows.iter()
            .all(|attributed| attributed.instance.gem.as_str() == "sample.gamedata"),
        "every schema is attributed to the contributing gem"
    );
}

#[test]
fn the_catalog_builds_around_the_composed_manager_registry() {
    let composer = compose(GemTargetRole::ProjectHost);
    let managers = composer
        .registries()
        .get::<GameDataManagerShape>()
        .expect("managers were registered")
        .entries()
        .copied()
        .collect::<Vec<_>>();

    let target = ProviderTarget::table(TABLE);
    let sources = [ManagerProjectionSource::new(target, SchemaHash(7))];
    let catalog =
        build_manager_catalog(ManagerCatalogInput::new(&managers, &[TABLE], &[], &sources))
            .expect("composed manager catalog");

    let entry = catalog
        .entry(ManagerNodeId::Explicit("RegistryManager"))
        .expect("registered manager");
    assert_eq!(entry.source_targets(), &[target]);
}

#[test]
fn two_gems_claiming_one_manager_name_fail_composition() {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(Schema, az_gem_contract::ProductActivation::default())
        .unwrap();
    composer
        .add(Shadow, az_gem_contract::ProductActivation::default())
        .unwrap();

    let ComposeError::Duplicate {
        registry,
        key,
        first,
        second,
    } = composer.finalize().unwrap_err()
    else {
        panic!("expected a duplicate registry key");
    };
    assert_eq!(registry, "gamedata-manager");
    assert_eq!(key, "RegistryManager");
    assert_eq!(first.gem.as_str(), "sample.gamedata");
    assert_eq!(second.gem.as_str(), "sample.shadow");
}

#[test]
fn two_gems_claiming_one_row_schema_fail_composition() {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(Schema, az_gem_contract::ProductActivation::default())
        .unwrap();
    composer
        .add(Twin, az_gem_contract::ProductActivation::default())
        .unwrap();

    let ComposeError::Duplicate {
        registry,
        key,
        first,
        second,
    } = composer.finalize().unwrap_err()
    else {
        panic!("expected a duplicate registry key");
    };
    assert_eq!(registry, "gamedata-row-schema");
    assert_eq!(key, "ItemData");
    assert_eq!(first.gem.as_str(), "sample.gamedata");
    assert_eq!(second.gem.as_str(), "sample.twin");
}

#[test]
fn withdrawing_a_gem_takes_both_gamedata_registries_with_it() {
    let mut composer = Composer::new(GemTargetRole::ProjectHost);
    composer
        .add(Schema, az_gem_contract::ProductActivation::default())
        .unwrap();

    let removed = composer.remove(GemId::new("sample.gamedata"), ContributionId::new("schema"));
    assert_eq!(removed, CATALOG.len() + 1, "every schema plus the manager");

    let report = composer.finalize().expect("composition is valid");
    assert!(report.entries.is_empty());
}

/// The `authoring` stanza's roles, in manifest order.
const AUTHORING_ROLES: [GemTargetRole; 1] = [GemTargetRole::ProjectHost];

/// The `builders` stanza's roles, in manifest order.
const BUILDERS_ROLES: [GemTargetRole; 2] =
    [GemTargetRole::AssetProcessor, GemTargetRole::AssetWorker];

fn compose_gem(contribution: impl Contribution, role: GemTargetRole) -> Composer {
    let mut composer = Composer::new(role);
    composer
        .add(contribution, az_gem_contract::ProductActivation::default())
        .expect("gamedata declares no host-capability floor");
    composer
}

#[test]
fn the_entry_items_carry_the_manifest_identities() {
    let authoring = authoring_contribution().descriptor();
    assert_eq!(authoring.gem, GEM);
    assert_eq!(authoring.contribution, AUTHORING);
    assert_eq!(authoring.roles, AUTHORING_ROLES);

    let builders = builders_contribution().descriptor();
    assert_eq!(builders.gem, GEM);
    assert_eq!(builders.contribution, BUILDERS);
    assert_eq!(builders.roles, BUILDERS_ROLES);
}

#[test]
fn the_builders_bundle_carries_the_whole_gamedata_builder_catalog() {
    for role in BUILDERS_ROLES {
        let composer = compose_gem(builders_contribution(), role);
        let report = composer.finalize().expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report
                .entries
                .iter()
                .all(|entry| entry.instance.gem == GEM && entry.instance.contribution == BUILDERS),
            "every entry is attributed to this contribution under `{role}`"
        );

        let registries = composer.registries();
        assert_eq!(
            registries
                .get::<AssetTypeRegistration>()
                .map(az_gem_contract::Registry::len),
            Some(gamedata::format::asset_types().len()),
            "the built table's catalog type is missing under `{role}`"
        );
        assert_eq!(
            registries
                .get::<ProductFormatRegistration>()
                .map(az_gem_contract::Registry::len),
            Some(gamedata::format::product_formats().len()),
            "the AZTBL byte contract is missing under `{role}`"
        );
        assert_eq!(
            registries
                .get::<SourceSchemaRegistration>()
                .map(az_gem_contract::Registry::len),
            Some(gamedata::authoring::source_schemas().len()),
            "the table source classifier is missing under `{role}`"
        );
    }
}

#[test]
fn the_builder_catalog_names_the_gems_own_formats() {
    let composer = compose_gem(builders_contribution(), GemTargetRole::AssetWorker);
    let report = composer.finalize().expect("composition is valid");

    let key = |registry: &str| {
        report
            .entries
            .iter()
            .filter(|entry| entry.registry == registry)
            .map(|entry| entry.key.as_str())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        key(ProductFormatRegistration::registry_name()),
        vec![gamedata::GAMEDATA_TABLE_FORMAT_ID.as_str()],
    );
    assert_eq!(
        key(SourceSchemaRegistration::registry_name()),
        vec![gamedata::authoring::GAMEDATA_TABLE_SOURCE_SCHEMA.as_str()],
    );
    assert_eq!(key(AssetTypeRegistration::registry_name()).len(), 1);
}

/// The `authoring` stanza is linkage, not content: `project-host` reads row
/// schemas and manager shapes that *projects* contribute through this gem's
/// registrar types, and the gem owns no entry of either. Holding it to empty is
/// what makes a registration added later a declared change rather than a
/// discovered one.
#[test]
fn the_authoring_bundle_registers_nothing_of_its_own() {
    for role in AUTHORING_ROLES {
        let report = compose_gem(authoring_contribution(), role)
            .finalize()
            .expect("composition is valid");
        assert!(report.refusals.is_empty(), "refused by `{role}`");
        assert!(
            report.entries.is_empty(),
            "the authoring stanza registers nothing under `{role}`, found {:?}",
            report.entries,
        );
    }
}
