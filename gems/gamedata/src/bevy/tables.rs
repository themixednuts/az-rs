//! Request-driven `GameData` table loading through the engine asset catalog.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use az_asset::AssetId;
use az_core::crc::Crc32;
use az_framework::asset::AssetCatalog;
use bevy::asset::LoadState;
use bevy::prelude::*;
use gamedata::format::{TABLE_PRODUCT_ROOT, family_directory};
use gamedata::game_system::{SchemaTable, System, TableId};
use gamedata::{GameDataError, GameDataSchemaRow, Row};

use super::GameDataAsset;

/// Loaded and requested `GameData` products for the active runtime asset catalog.
///
/// The resource indexes catalog paths without opening products. A typed query
/// requests only the named product. Family iteration explicitly requests every
/// product sharing the schema; products remain cached until released.
#[derive(Resource, Debug, Default)]
pub struct GameDataTables {
    system: System,
    catalog: Option<CatalogIndex>,
    catalog_error: Option<String>,
    requests: Mutex<BTreeSet<LoadRequest>>,
    products: BTreeMap<String, ProductLoad>,
    families: BTreeMap<String, FamilyLoadState>,
    loaded_paths: BTreeMap<String, LoadedIdentity>,
    pending_report: PendingLoadReport,
}

/// Cadence for reporting products that are still loading.
///
/// A `GameData` load carries no deadline of its own: a consumer keeps
/// re-requesting the same table every schedule run until the product resolves.
/// Without this report an unresolvable product is indistinguishable from a slow
/// one, and the process spins mute forever.
const PENDING_LOAD_REPORT_INTERVAL: Duration = Duration::from_secs(15);

/// Upper bound on the number of paths named in one pending-load report.
const PENDING_LOAD_REPORT_SAMPLE: usize = 8;

/// Time accumulated while at least one product is still loading.
#[derive(Debug, Default)]
struct PendingLoadReport {
    waited: Duration,
}

#[derive(Debug, Default)]
struct CatalogIndex {
    families: BTreeMap<String, Vec<String>>,
    by_asset_id: BTreeMap<AssetId, String>,
    by_table_id: BTreeMap<(String, TableId), String>,
    products: BTreeMap<String, CatalogProduct>,
}

#[derive(Debug, Clone)]
struct CatalogProduct {
    asset_id: AssetId,
    table_id: TableId,
    family: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum LoadRequest {
    Family(String),
    Product(String),
}

#[derive(Debug)]
struct ProductLoad {
    handle: Handle<GameDataAsset>,
    state: ProductLoadState,
}

#[derive(Debug, Clone)]
enum ProductLoadState {
    Loading,
    Ready,
    Failed(String),
}

#[derive(Debug, Clone)]
enum FamilyLoadState {
    Loading(Vec<String>),
    Ready(Vec<String>),
    Failed(String),
}

#[derive(Debug, Clone, Copy)]
struct LoadedIdentity {
    table_name_crc: u32,
    row_type_crc: u32,
}

impl GameDataTables {
    /// Access one physical table by its authored name.
    ///
    /// The first call queues only that table product and returns a loading
    /// error. Bevy systems should retry on later schedule runs.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::LoadingTableProduct`] the first time, having
    /// queued the product; [`GameDataError::Decode`] when the active catalog
    /// failed to index, names no such product, or the product's row CRC does
    /// not match `R`; and whatever [`SchemaTable`] materialization returns when
    /// the loaded table does not satisfy `R`'s merged row schema.
    pub fn schema_table<R>(&self, table_name: &str) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        match self.system.schema_table::<R>(table_name) {
            Ok(table) => Ok(table),
            Err(GameDataError::MissingTable { .. }) => {
                let path = self.request_table::<R>(TableId::from_name(table_name), table_name)?;
                Err(GameDataError::LoadingTableProduct { path })
            }
            Err(error) => Err(error),
        }
    }

    /// Access one physical table by the lowercase CRC of its authored name.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::LoadingTableProduct`] the first time, having
    /// queued the product; [`GameDataError::Decode`] when the active catalog
    /// failed to index, names no such product, or the product's row CRC does
    /// not match `R`; and whatever [`SchemaTable`] materialization returns when
    /// the loaded table does not satisfy `R`'s merged row schema.
    pub fn schema_table_by_crc<R>(
        &self,
        table_name_crc: impl Into<Crc32>,
    ) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        let table_name_crc = table_name_crc.into().value();
        match self.system.schema_table_by_crc::<R>(table_name_crc) {
            Ok(table) => Ok(table),
            Err(GameDataError::MissingTable { .. }) => {
                let logical_name = format!("crc:{table_name_crc:#010x}");
                let path = self
                    .request_table::<R>(TableId::from_crc(table_name_crc.into()), &logical_name)?;
                Err(GameDataError::LoadingTableProduct { path })
            }
            Err(error) => Err(error),
        }
    }

    /// Access one table by its exact product path in the active catalog.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::LoadingTableProduct`] the first time, having
    /// queued the product; [`GameDataError::Decode`] when the active catalog
    /// failed to index, names no such product, or the product's row CRC does
    /// not match `R`; and whatever [`SchemaTable`] materialization returns when
    /// the loaded table does not satisfy `R`'s merged row schema.
    pub fn schema_table_at<R>(&self, product_path: &str) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        let path = normalize_path(product_path);
        self.validate_product_family::<R>(&path)?;
        if let Some(identity) = self.loaded_paths.get(&path) {
            if identity.row_type_crc != <R as Row>::CRC {
                return Err(GameDataError::Decode(format!(
                    "GameData product `{path}` has row CRC {:#010x}, expected {:#010x}",
                    identity.row_type_crc,
                    <R as Row>::CRC,
                )));
            }
            return self
                .system
                .schema_table_by_crc::<R>(identity.table_name_crc);
        }
        self.request_product(&path)?;
        Err(GameDataError::LoadingTableProduct { path })
    }

    /// Access one table by its stable engine asset id.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::Decode`] when the active catalog does not map
    /// `asset_id` to a product path, plus anything
    /// [`Self::schema_table_at`] returns for that path.
    pub fn schema_table_by_asset_id<R>(
        &self,
        asset_id: AssetId,
    ) -> Result<SchemaTable<R>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        let path = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.by_asset_id.get(&asset_id))
            .ok_or_else(|| {
                GameDataError::Decode(format!("unknown GameData asset id {asset_id}"))
            })?;
        self.schema_table_at::<R>(path)
    }

    /// Access every physical table in one merged row-schema family.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataError::LoadingTableFamily`] the first time, having
    /// queued the family, [`GameDataError::TableFamilyLoadFailed`] once a
    /// product in the family failed to load, and whatever table
    /// materialization returns when a loaded table does not satisfy `R`.
    pub fn schema_tables<R>(&self) -> Result<Vec<SchemaTable<R>>, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        let family = schema_family::<R>();
        match self.families.get(&family) {
            Some(FamilyLoadState::Ready(_)) => self.system.schema_tables::<R>(),
            Some(FamilyLoadState::Failed(error)) => Err(GameDataError::TableFamilyLoadFailed {
                schema: R::SCHEMA.name().to_owned(),
                error: error.clone(),
            }),
            Some(FamilyLoadState::Loading(_)) | None => {
                self.queue_request(LoadRequest::Family(family));
                Err(GameDataError::LoadingTableFamily {
                    schema: R::SCHEMA.name().to_owned(),
                })
            }
        }
    }

    /// Release one exact table product and its typed projection.
    /// Existing owning table and row handles remain valid.
    ///
    /// # Panics
    ///
    /// Panics if the request queue mutex is poisoned, which means an earlier
    /// `GameData` system panicked while holding it.
    pub fn release_product(&mut self, product_path: &str) -> bool {
        let path = normalize_path(product_path);
        let family = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.products.get(&path))
            .map(|product| product.family.clone());
        if let Some(family) = &family {
            self.families.remove(family);
        }
        self.requests
            .lock()
            .expect("GameData request queue lock poisoned")
            .retain(|request| match request {
                LoadRequest::Product(requested) => requested != &path,
                LoadRequest::Family(requested) => family.as_ref() != Some(requested),
            });
        let removed_product = self.products.remove(&path).is_some();
        let Some(identity) = self.loaded_paths.remove(&path) else {
            return removed_product;
        };
        self.system
            .remove_table(identity.table_name_crc, identity.row_type_crc);
        true
    }

    /// Release one exact table selected by its authored table identity within
    /// the requested merged row-schema family.
    pub fn release_table<R>(&mut self, table_id: TableId) -> bool
    where
        R: GameDataSchemaRow,
    {
        let family = schema_family::<R>();
        let path = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.by_table_id.get(&(family, table_id)))
            .cloned();
        path.is_some_and(|path| self.release_product(&path))
    }

    /// Release one exact table selected by its stable engine asset id.
    pub fn release_asset(&mut self, asset_id: AssetId) -> bool {
        let path = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.by_asset_id.get(&asset_id))
            .cloned();
        path.is_some_and(|path| self.release_product(&path))
    }

    /// Release raw products and typed projections for one schema family.
    /// Existing owning table and row handles remain valid.
    pub fn release_schema<R>(&mut self)
    where
        R: GameDataSchemaRow,
    {
        let family = schema_family::<R>();
        let paths = self
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.families.get(&family))
            .cloned()
            .unwrap_or_default();
        for path in &paths {
            self.release_product(path);
        }
        self.families.remove(&family);
        self.system.remove_row_type(<R as Row>::CRC);
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.system.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.system.is_empty()
    }

    fn request_table<R>(
        &self,
        table_id: TableId,
        logical_name: &str,
    ) -> Result<String, GameDataError>
    where
        R: GameDataSchemaRow,
    {
        if let Some(error) = &self.catalog_error {
            return Err(GameDataError::Decode(error.clone()));
        }
        let Some(catalog) = &self.catalog else {
            return Err(GameDataError::LoadingTableProduct {
                path: logical_name.to_owned(),
            });
        };
        let family = schema_family::<R>();
        let path = catalog
            .by_table_id
            .get(&(family, table_id))
            .cloned()
            .ok_or_else(|| GameDataError::MissingTable {
                logical_name: logical_name.to_owned(),
            })?;
        self.request_product(&path)?;
        Ok(path)
    }

    fn request_product(&self, path: &str) -> Result<(), GameDataError> {
        if let Some(error) = &self.catalog_error {
            return Err(GameDataError::Decode(error.clone()));
        }
        let Some(catalog) = &self.catalog else {
            return Err(GameDataError::LoadingTableProduct {
                path: path.to_owned(),
            });
        };
        if !catalog.products.contains_key(path) {
            return Err(GameDataError::Decode(format!(
                "GameData product `{path}` is not present in the active asset catalog"
            )));
        }
        match self.products.get(path).map(|product| &product.state) {
            Some(ProductLoadState::Failed(error)) => Err(GameDataError::Decode(format!(
                "GameData product `{path}` failed to load: {error}"
            ))),
            Some(ProductLoadState::Loading | ProductLoadState::Ready) => Ok(()),
            None => {
                self.queue_request(LoadRequest::Product(path.to_owned()));
                Ok(())
            }
        }
    }

    fn validate_product_family<R>(&self, path: &str) -> Result<(), GameDataError>
    where
        R: GameDataSchemaRow,
    {
        if let Some(error) = &self.catalog_error {
            return Err(GameDataError::Decode(error.clone()));
        }
        let Some(catalog) = &self.catalog else {
            return Err(GameDataError::LoadingTableProduct {
                path: path.to_owned(),
            });
        };
        let product = catalog.products.get(path).ok_or_else(|| {
            GameDataError::Decode(format!(
                "GameData product `{path}` is not present in the active asset catalog"
            ))
        })?;
        let expected = schema_family::<R>();
        if product.family != expected {
            return Err(GameDataError::Decode(format!(
                "GameData product `{path}` belongs to schema family `{}`, expected `{expected}`",
                product.family
            )));
        }
        Ok(())
    }

    fn queue_request(&self, request: LoadRequest) {
        self.requests
            .lock()
            .expect("GameData request queue lock poisoned")
            .insert(request);
    }

    /// Record a product load failure and surface it on the log.
    ///
    /// Every transition into [`ProductLoadState::Failed`] reports here, so a
    /// product that can never resolve is never mistaken for a slow one.
    fn fail_product(&mut self, path: &str, error: String) {
        tracing::error!(product = %path, error = %error, "GameData product load failed");
        if let Some(load) = self.products.get_mut(path) {
            load.state = ProductLoadState::Failed(error);
        }
    }

    /// Products still waiting on the asset server.
    fn pending_products(&self) -> impl Iterator<Item = &str> {
        self.products
            .iter()
            .filter(|(_, product)| matches!(product.state, ProductLoadState::Loading))
            .map(|(path, _)| path.as_str())
    }
}

pub fn index_gamedata_catalog(
    catalog: Option<Res<AssetCatalog>>,
    mut tables: ResMut<GameDataTables>,
) {
    if tables.catalog.is_some() || tables.catalog_error.is_some() {
        return;
    }
    let Some(catalog) = catalog else {
        tables.catalog_error = Some("GameData loading requires AssetCatalog".to_owned());
        return;
    };

    let mut index = CatalogIndex::default();
    for entry in catalog.entries_by_asset_type(gamedata::GAMEDATA_TABLE_ASSET_TYPE) {
        let Some(path) = entry.relative_path().to_str() else {
            tables.catalog_error = Some(format!(
                "GameData catalog path is not valid UTF-8: {}",
                entry.relative_path().display()
            ));
            return;
        };
        let path = normalize_path(path);
        let Some(family) = product_family(&path) else {
            tables.catalog_error = Some(format!(
                "GameData product `{path}` is outside tables/<schema>/"
            ));
            return;
        };
        let asset_id = entry.asset_id();
        let table_id = TableId::from_crc(asset_id.sub_id.into());
        if asset_id.sub_id == 0 {
            tables.catalog_error = Some(format!(
                "GameData product `{path}` has no authored table identity in its asset sub-id"
            ));
            return;
        }
        let family_table_id = (family.to_owned(), table_id);
        if let Some(existing) = index.by_table_id.insert(family_table_id, path.clone()) {
            tables.catalog_error = Some(format!(
                "GameData products `{existing}` and `{path}` have duplicate table identity {:#010x} within schema family `{family}`",
                table_id.value(),
            ));
            return;
        }
        if let Some(existing) = index.by_asset_id.insert(asset_id, path.clone()) {
            tables.catalog_error = Some(format!(
                "GameData products `{existing}` and `{path}` have duplicate asset id {asset_id}"
            ));
            return;
        }
        if index
            .products
            .insert(
                path.clone(),
                CatalogProduct {
                    asset_id,
                    table_id,
                    family: family.to_owned(),
                },
            )
            .is_some()
        {
            tables.catalog_error = Some(format!(
                "duplicate normalized GameData product path `{path}`"
            ));
            return;
        }
        index
            .families
            .entry(family.to_owned())
            .or_default()
            .push(path.clone());
    }
    for paths in index.families.values_mut() {
        paths.sort_unstable();
        paths.dedup();
    }
    tracing::info!(
        products = index.products.len(),
        "indexed GameData catalog products"
    );
    tables.catalog = Some(index);
}

// `Res` is an owned Bevy system parameter: borrowing it here would stop this
// function satisfying `IntoSystem` and it could no longer be registered.
#[allow(clippy::needless_pass_by_value)]
pub fn queue_requested_products(
    asset_server: Res<AssetServer>,
    mut tables: ResMut<GameDataTables>,
) {
    let requests = {
        let mut requests = tables
            .requests
            .lock()
            .expect("GameData request queue lock poisoned");
        std::mem::take(&mut *requests)
    };
    for request in requests {
        match request {
            LoadRequest::Family(family) => {
                if tables.families.contains_key(&family) {
                    continue;
                }
                let Some(paths) = tables
                    .catalog
                    .as_ref()
                    .and_then(|catalog| catalog.families.get(&family))
                    .cloned()
                else {
                    tracing::error!(
                        schema_family = %family,
                        "GameData schema family has no products in the active catalog"
                    );
                    tables.families.insert(
                        family,
                        FamilyLoadState::Failed("no products in active catalog".to_owned()),
                    );
                    continue;
                };
                for path in &paths {
                    tables
                        .products
                        .entry(path.clone())
                        .or_insert_with(|| ProductLoad {
                            handle: asset_server.load::<GameDataAsset>(path.clone()),
                            state: ProductLoadState::Loading,
                        });
                }
                tracing::info!(schema_family = %family, products = paths.len(), "queued GameData schema family");
                tables
                    .families
                    .insert(family, FamilyLoadState::Loading(paths));
            }
            LoadRequest::Product(path) => {
                tables
                    .products
                    .entry(path.clone())
                    .or_insert_with(|| ProductLoad {
                        handle: asset_server.load::<GameDataAsset>(path),
                        state: ProductLoadState::Loading,
                    });
            }
        }
    }
}

// `Res` is an owned Bevy system parameter: borrowing these would stop this
// function satisfying `IntoSystem` and it could no longer be registered.
#[allow(clippy::needless_pass_by_value)]
pub fn collect_loaded_products(
    assets: Res<Assets<GameDataAsset>>,
    asset_server: Res<AssetServer>,
    time: Res<Time>,
    mut tables: ResMut<GameDataTables>,
) {
    let mut ready = Vec::new();
    for (path, product) in &mut tables.products {
        if !matches!(product.state, ProductLoadState::Loading) {
            continue;
        }
        match asset_server.load_state(product.handle.id()) {
            LoadState::Loaded => {
                if let Some(asset) = assets.get(&product.handle) {
                    ready.push((path.clone(), asset.table.clone()));
                    product.state = ProductLoadState::Ready;
                }
            }
            LoadState::Failed(error) => {
                tracing::error!(
                    product = %path,
                    error = %error,
                    "GameData product load failed"
                );
                product.state = ProductLoadState::Failed(error.to_string());
            }
            _ => {}
        }
    }

    for (path, asset) in ready {
        let header = asset.header();
        let Some(product) = tables
            .catalog
            .as_ref()
            .and_then(|catalog| catalog.products.get(&path))
        else {
            tables.fail_product(
                &path,
                "product disappeared from the active GameData catalog".to_owned(),
            );
            continue;
        };
        if header.table_name_crc != product.table_id.value() {
            let error = format!(
                "product `{path}` header table CRC {:#010x} does not match catalog identity {:#010x} (asset {})",
                header.table_name_crc,
                product.table_id.value(),
                product.asset_id,
            );
            tables.fail_product(&path, error);
            continue;
        }
        if let Err(error) = tables.system.insert(asset) {
            tables.fail_product(&path, error.to_string());
            continue;
        }
        tables.loaded_paths.insert(
            path,
            LoadedIdentity {
                table_name_crc: header.table_name_crc,
                row_type_crc: header.row_type_crc,
            },
        );
    }

    let family_updates = tables
        .families
        .iter()
        .filter_map(|(family, state)| {
            let FamilyLoadState::Loading(paths) = state else {
                return None;
            };
            let failure = paths.iter().find_map(|path| {
                let ProductLoadState::Failed(error) = &tables.products.get(path)?.state else {
                    return None;
                };
                Some(format!("product `{path}`: {error}"))
            });
            if let Some(error) = failure {
                return Some((family.clone(), FamilyLoadState::Failed(error)));
            }
            paths
                .iter()
                .all(|path| {
                    matches!(
                        tables.products.get(path).map(|product| &product.state),
                        Some(ProductLoadState::Ready)
                    )
                })
                .then(|| (family.clone(), FamilyLoadState::Ready(paths.clone())))
        })
        .collect::<Vec<_>>();
    for (family, state) in family_updates {
        match &state {
            FamilyLoadState::Ready(paths) => {
                tracing::info!(schema_family = %family, products = paths.len(), "loaded GameData schema family");
            }
            FamilyLoadState::Failed(error) => {
                tracing::error!(schema_family = %family, error = %error, "GameData schema family failed to load");
            }
            FamilyLoadState::Loading(_) => {}
        }
        tables.families.insert(family, state);
    }

    report_pending_loads(&mut tables, time.delta());
}

/// Report the products that are still loading, at a fixed cadence.
///
/// A `GameData` consumer retries its request every schedule run, so a product
/// that never resolves produces no output at all. Naming the pending set is the
/// difference between a startup that is working and one that is stuck.
fn report_pending_loads(tables: &mut GameDataTables, delta: Duration) {
    let pending = tables.pending_products().count();
    if pending == 0 {
        tables.pending_report.waited = Duration::ZERO;
        return;
    }
    tables.pending_report.waited += delta;
    if tables.pending_report.waited < PENDING_LOAD_REPORT_INTERVAL {
        return;
    }
    tables.pending_report.waited = Duration::ZERO;
    let sample = tables
        .pending_products()
        .take(PENDING_LOAD_REPORT_SAMPLE)
        .collect::<Vec<_>>();
    tracing::warn!(
        pending,
        waiting_for = ?sample,
        "GameData products are still loading"
    );
}

fn schema_family<R>() -> String
where
    R: GameDataSchemaRow,
{
    family_directory(R::SCHEMA.name())
}

fn product_family(path: &str) -> Option<&str> {
    let path = path.strip_prefix(TABLE_PRODUCT_ROOT)?.strip_prefix('/')?;
    path.split_once('/').map(|(family, _)| family)
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use super::*;
    use crate::descriptor::RowSchemaDescriptor;
    use crate::game_system::SchemaRowRef;
    use crate::{GameDataRow, GameDataSchemaRow, SchemaHash, TableAsset};

    #[derive(Debug, PartialEq, Eq)]
    struct FixtureSchemaRow;

    impl Row for FixtureSchemaRow {
        const NAME: &'static str = "FixtureSchema";
        const CRC: u32 = 0x1020_3040;
    }

    impl GameDataRow for FixtureSchemaRow {
        const KEY_FIELD_NAMES: &'static [&'static str] = &[];
    }

    impl GameDataSchemaRow for FixtureSchemaRow {
        const SCHEMA: RowSchemaDescriptor = RowSchemaDescriptor::new(Self::NAME, &[]);

        fn decode(_row: SchemaRowRef<'_, '_, Self>) -> Result<Self, GameDataError> {
            Ok(Self)
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct OtherFixtureSchemaRow;

    impl Row for OtherFixtureSchemaRow {
        const NAME: &'static str = "OtherFixtureSchema";
        const CRC: u32 = 0x5060_7080;
    }

    impl GameDataRow for OtherFixtureSchemaRow {
        const KEY_FIELD_NAMES: &'static [&'static str] = &[];
    }

    impl GameDataSchemaRow for OtherFixtureSchemaRow {
        const SCHEMA: RowSchemaDescriptor = RowSchemaDescriptor::new(Self::NAME, &[]);

        fn decode(_row: SchemaRowRef<'_, '_, Self>) -> Result<Self, GameDataError> {
            Ok(Self)
        }
    }

    /// A schema whose name every *Rust-module* naming rule renders differently:
    /// `match` is a keyword, so a module namer produces `match_`. Product
    /// directories are not Rust, so the family is plain `match` — this fixture
    /// is what makes the routing test below able to fail.
    #[derive(Debug, PartialEq, Eq)]
    struct KeywordSchemaRow;

    impl Row for KeywordSchemaRow {
        const NAME: &'static str = "Match";
        const CRC: u32 = 0x90a0_b0c0;
    }

    impl GameDataRow for KeywordSchemaRow {
        const KEY_FIELD_NAMES: &'static [&'static str] = &[];
    }

    impl GameDataSchemaRow for KeywordSchemaRow {
        const SCHEMA: RowSchemaDescriptor = RowSchemaDescriptor::new(Self::NAME, &[]);

        fn decode(_row: SchemaRowRef<'_, '_, Self>) -> Result<Self, GameDataError> {
            Ok(Self)
        }
    }

    fn fixture_catalog() -> CatalogIndex {
        let alpha_path = "tables/fixture_schema/alpha.aztbl".to_owned();
        let beta_path = "tables/fixture_schema/beta.aztbl".to_owned();
        let alpha_id = TableId::from_name("Alpha");
        let beta_id = TableId::from_name("Beta");
        let alpha_asset = AssetId::new(Uuid::new_v4(), alpha_id.value());
        let beta_asset = AssetId::new(Uuid::new_v4(), beta_id.value());
        CatalogIndex {
            families: BTreeMap::from([(
                "fixture_schema".to_owned(),
                vec![alpha_path.clone(), beta_path.clone()],
            )]),
            by_asset_id: BTreeMap::from([
                (alpha_asset, alpha_path.clone()),
                (beta_asset, beta_path.clone()),
            ]),
            by_table_id: BTreeMap::from([
                (("fixture_schema".to_owned(), alpha_id), alpha_path.clone()),
                (("fixture_schema".to_owned(), beta_id), beta_path.clone()),
            ]),
            products: BTreeMap::from([
                (
                    alpha_path,
                    CatalogProduct {
                        asset_id: alpha_asset,
                        table_id: alpha_id,
                        family: "fixture_schema".to_owned(),
                    },
                ),
                (
                    beta_path,
                    CatalogProduct {
                        asset_id: beta_asset,
                        table_id: beta_id,
                        family: "fixture_schema".to_owned(),
                    },
                ),
            ]),
        }
    }

    fn install_loaded(tables: &mut GameDataTables, path: &str, table_name: &str) {
        let asset = Arc::new(
            TableAsset::from_bytes(crate::table::fixture_table_asset_bytes(
                SchemaHash(1),
                table_name,
                FixtureSchemaRow::CRC,
                1,
            ))
            .expect("fixture table"),
        );
        let header = asset.header();
        tables.system.insert(asset).expect("insert fixture table");
        tables.products.insert(
            path.to_owned(),
            ProductLoad {
                handle: Handle::default(),
                state: ProductLoadState::Ready,
            },
        );
        tables.loaded_paths.insert(
            path.to_owned(),
            LoadedIdentity {
                table_name_crc: header.table_name_crc,
                row_type_crc: header.row_type_crc,
            },
        );
    }

    #[test]
    fn family_routing_requests_the_directory_the_gem_exports() {
        let tables = GameDataTables::default();

        tables
            .schema_tables::<KeywordSchemaRow>()
            .expect_err("first family access queues the family");

        let requested = tables.requests.lock().expect("request queue").clone();
        assert_eq!(
            requested,
            BTreeSet::from([LoadRequest::Family(family_directory(
                KeywordSchemaRow::SCHEMA.name()
            ))]),
            "family routing must ask for the directory `gamedata::format::family_directory` names"
        );
        assert_eq!(
            requested,
            BTreeSet::from([LoadRequest::Family("match".to_owned())]),
            "a product directory is not a Rust module, so a keyword schema keeps its plain name"
        );
    }

    #[test]
    fn schema_family_matches_authored_product_layout() {
        assert_eq!(family_directory("AITargetingData"), "ai_targeting_data");
        assert_eq!(family_directory("2025Data"), "_2025data");
        assert_eq!(
            product_family("tables/damage_data/damage_table.aztbl"),
            Some("damage_data")
        );
    }

    #[test]
    fn named_lookup_requests_only_the_exact_catalog_product() {
        let tables = GameDataTables {
            catalog: Some(fixture_catalog()),
            ..Default::default()
        };

        let error = tables
            .schema_table::<FixtureSchemaRow>("Alpha")
            .expect_err("first access queues product");
        assert!(matches!(
            error,
            GameDataError::LoadingTableProduct { ref path }
                if path == "tables/fixture_schema/alpha.aztbl"
        ));
        assert_eq!(
            *tables.requests.lock().expect("request queue"),
            BTreeSet::from([LoadRequest::Product(
                "tables/fixture_schema/alpha.aztbl".to_owned()
            )])
        );
    }

    #[test]
    fn table_crc_is_scoped_to_its_schema_family() {
        let table_id = TableId::from_name("Shared");
        let fixture_path = "tables/fixture_schema/shared.aztbl".to_owned();
        let other_path = "tables/other_fixture_schema/shared.aztbl".to_owned();
        let fixture_asset = AssetId::new(Uuid::new_v4(), table_id.value());
        let other_asset = AssetId::new(Uuid::new_v4(), table_id.value());
        let tables = GameDataTables {
            catalog: Some(CatalogIndex {
                families: BTreeMap::from([
                    ("fixture_schema".to_owned(), vec![fixture_path.clone()]),
                    ("other_fixture_schema".to_owned(), vec![other_path.clone()]),
                ]),
                by_asset_id: BTreeMap::from([
                    (fixture_asset, fixture_path.clone()),
                    (other_asset, other_path.clone()),
                ]),
                by_table_id: BTreeMap::from([
                    (
                        ("fixture_schema".to_owned(), table_id),
                        fixture_path.clone(),
                    ),
                    (
                        ("other_fixture_schema".to_owned(), table_id),
                        other_path.clone(),
                    ),
                ]),
                products: BTreeMap::from([
                    (
                        fixture_path.clone(),
                        CatalogProduct {
                            asset_id: fixture_asset,
                            table_id,
                            family: "fixture_schema".to_owned(),
                        },
                    ),
                    (
                        other_path.clone(),
                        CatalogProduct {
                            asset_id: other_asset,
                            table_id,
                            family: "other_fixture_schema".to_owned(),
                        },
                    ),
                ]),
            }),
            ..Default::default()
        };

        assert!(matches!(
            tables.schema_table::<FixtureSchemaRow>("Shared"),
            Err(GameDataError::LoadingTableProduct { path }) if path == fixture_path
        ));
        assert!(matches!(
            tables.schema_table::<OtherFixtureSchemaRow>("Shared"),
            Err(GameDataError::LoadingTableProduct { path }) if path == other_path
        ));
    }

    #[test]
    fn exact_release_preserves_handles_and_allows_reload() {
        let path = "tables/fixture_schema/alpha.aztbl";
        let mut tables = GameDataTables {
            catalog: Some(fixture_catalog()),
            ..Default::default()
        };
        install_loaded(&mut tables, path, "Alpha");

        let table = tables
            .schema_table::<FixtureSchemaRow>("Alpha")
            .expect("loaded projection");
        let row = table.row_handles().next().expect("owning row handle");
        assert!(tables.release_table::<FixtureSchemaRow>(TableId::from_name("Alpha")));
        assert!(tables.is_empty());
        assert_eq!(table.len(), 1);
        assert_eq!(row.table_name(), "Alpha");

        assert!(matches!(
            tables.schema_table::<FixtureSchemaRow>("Alpha"),
            Err(GameDataError::LoadingTableProduct { path: ref queued }) if queued == path
        ));
        install_loaded(&mut tables, path, "Alpha");
        assert_eq!(
            tables
                .schema_table::<FixtureSchemaRow>("Alpha")
                .expect("reloaded projection")
                .len(),
            1
        );
    }

    fn install_loading(tables: &mut GameDataTables, path: &str) {
        tables.products.insert(
            path.to_owned(),
            ProductLoad {
                handle: Handle::default(),
                state: ProductLoadState::Loading,
            },
        );
    }

    #[test]
    fn a_load_that_never_resolves_reports_its_pending_set_on_a_cadence() {
        let path = "tables/fixture_schema/alpha.aztbl";
        let mut tables = GameDataTables::default();
        install_loading(&mut tables, path);

        // Short of the cadence the report stays quiet but keeps the clock.
        let step = PENDING_LOAD_REPORT_INTERVAL / 3;
        report_pending_loads(&mut tables, step);
        assert_eq!(tables.pending_report.waited, step);
        report_pending_loads(&mut tables, step);
        assert_eq!(tables.pending_report.waited, step * 2);

        // Crossing the cadence reports and rearms, so the wait keeps speaking
        // for as long as it lasts instead of falling silent after one line.
        report_pending_loads(&mut tables, step);
        assert_eq!(tables.pending_report.waited, Duration::ZERO);
        assert_eq!(tables.pending_products().collect::<Vec<_>>(), [path]);

        report_pending_loads(&mut tables, PENDING_LOAD_REPORT_INTERVAL);
        assert_eq!(tables.pending_report.waited, Duration::ZERO);
    }

    #[test]
    fn a_resolved_load_disarms_the_pending_report() {
        let path = "tables/fixture_schema/alpha.aztbl";
        let mut tables = GameDataTables::default();
        install_loading(&mut tables, path);
        report_pending_loads(&mut tables, PENDING_LOAD_REPORT_INTERVAL / 2);
        assert!(tables.pending_report.waited > Duration::ZERO);

        install_loaded(&mut tables, path, "Alpha");
        report_pending_loads(&mut tables, PENDING_LOAD_REPORT_INTERVAL);
        assert_eq!(tables.pending_report.waited, Duration::ZERO);
        assert_eq!(tables.pending_products().count(), 0);
    }

    #[test]
    fn a_failed_product_stops_being_pending_and_surfaces_to_callers() {
        let path = "tables/fixture_schema/alpha.aztbl".to_owned();
        let mut tables = GameDataTables {
            catalog: Some(fixture_catalog()),
            ..GameDataTables::default()
        };
        install_loading(&mut tables, &path);

        tables.fail_product(&path, "decode blew up".to_owned());

        assert_eq!(tables.pending_products().count(), 0);
        report_pending_loads(&mut tables, PENDING_LOAD_REPORT_INTERVAL);
        assert_eq!(tables.pending_report.waited, Duration::ZERO);
        assert!(matches!(
            tables.request_product(&path),
            Err(GameDataError::Decode(ref message)) if message.contains("decode blew up")
        ));
    }
}
