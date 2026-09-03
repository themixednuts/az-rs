use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use az_gem_contract::{Registries, Registry};
use az_proto_asset::WorkspaceSourceFileRef;
use az_proto_core::{SideChannelHandle, write_content_addressed_staging_file};
use az_proto_project::{
    GAMEDATA_CATALOG_SNAPSHOT_VERSION, GameDataCatalogDiagnostic, GameDataCatalogSnapshot,
    GameDataKeyPolicy, GameDataManagerCatalogEntry, GameDataManagerInput as ProtoManagerInput,
    GameDataManagerNodeRef, GameDataProjectionTransform, GameDataProviderTarget, GameDataRowFilter,
    GameDataSecondaryIndex, GameDataTableDescriptor, encode_gamedata_catalog_snapshot,
};
use gamedata::authoring::{
    ManagerCatalogDiagnostic, ManagerCatalogInput, ManagerGraphError, ManagerNode, ManagerNodeId,
    ManagerProjectionSource, build_manager_catalog,
};
use gamedata::manager::{
    DuplicateKeyPolicy, GameDataManagerInput, GameDataManagerShape, KeyKind, KeyPolicy,
    KeyTransform, ManagerShapeKind, ProductInputFormat, ProjectionTransformKind, ProviderTarget,
    RowFilterPredicate, SecondaryIndexKeyKind, SecondaryIndexStorage, SourceSchemaRole,
    TableInputDescriptor,
};
use gamedata::release::SchemaHash;
use gamedata::{RowSchema, RowSchemaDescriptor, row_schema_hash};
use thiserror::Error;
use tracing::{info, instrument};

#[derive(Debug, Error)]
pub enum GameDataCatalogPublishError {
    #[error("failed to resolve GameData manager catalog: {0}")]
    Resolve(#[from] ManagerGraphError),

    #[error("failed to encode GameData catalog snapshot: {0}")]
    Encode(#[from] capnp::Error),

    #[error("failed to write GameData catalog side-channel file {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("system clock is before Unix epoch")]
    ClockBeforeUnixEpoch,
}

#[derive(Debug, Clone)]
pub struct GameDataCatalogSideChannel {
    root: PathBuf,
}

impl GameDataCatalogSideChannel {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[must_use]
    pub fn default_root() -> PathBuf {
        std::env::temp_dir().join("azoth").join("gamedata-catalog")
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Compose the registered `GameData` catalog and publish it.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataCatalogPublishError::ClockBeforeUnixEpoch`] when the
    /// system clock predates the Unix epoch,
    /// [`GameDataCatalogPublishError::Resolve`] when the manager catalog does
    /// not resolve, and any error [`Self::write_snapshot`] returns.
    #[instrument(skip_all, fields(root = %self.root.display()))]
    pub fn write_registered_snapshot(
        &self,
        registries: &Registries,
        discovered_tables: &[DiscoveredGameDataTable],
    ) -> Result<GameDataCatalogSideChannelFile, GameDataCatalogPublishError> {
        let snapshot = compose_registered_gamedata_catalog_snapshot(
            registries,
            discovered_tables,
            now_unix_ms()?,
        )?;
        info!(
            tables = snapshot.tables.len(),
            families = snapshot.families.len(),
            managers = snapshot.managers.len(),
            diagnostics = snapshot.diagnostics.len(),
            "publishing GameData catalog snapshot"
        );
        self.write_snapshot(&snapshot)
    }

    /// Publish `snapshot` as a content-addressed side-channel catalog file.
    ///
    /// # Errors
    ///
    /// Returns [`GameDataCatalogPublishError::Encode`] when the snapshot cannot
    /// be serialized, or [`GameDataCatalogPublishError::Write`] when the
    /// staging file cannot be written under [`Self::root`].
    #[instrument(skip_all, fields(root = %self.root.display()))]
    pub fn write_snapshot(
        &self,
        snapshot: &GameDataCatalogSnapshot,
    ) -> Result<GameDataCatalogSideChannelFile, GameDataCatalogPublishError> {
        let bytes = encode_gamedata_catalog_snapshot(snapshot)?;
        let written = write_content_addressed_staging_file(&self.root, "gamedata-catalog", &bytes)
            .map_err(|error| GameDataCatalogPublishError::Write {
                path: error.path,
                source: error.source,
            })?;

        let handle = SideChannelHandle::staging_file(
            written.path.to_string_lossy(),
            written.byte_length,
            written.content_hash,
            std::env::consts::OS,
        );

        info!(
            path = %written.path.display(),
            byte_length = written.byte_length,
            "wrote GameData catalog side-channel"
        );

        Ok(GameDataCatalogSideChannelFile {
            path: written.path,
            handle,
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for GameDataCatalogSideChannel {
    fn default() -> Self {
        Self::new(Self::default_root())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameDataCatalogSideChannelFile {
    pub path: PathBuf,
    pub handle: SideChannelHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredGameDataTable {
    pub name: String,
    pub row_type: String,
    pub source_root: String,
    /// Asset Processor portable root key, not a UI-facing root label.
    pub source_root_key: String,
    /// File codec dispatch key, deliberately distinct from `row_type`.
    pub source_schema_type: String,
    pub source_path: String,
    pub document_id: String,
    pub owner: String,
    pub row_count: u64,
}

/// Composes the editor-facing catalog from one host's composed `GameData`
/// registries.
///
/// Table families have no registry of their own: a family reaches the catalog
/// only as an input of a composed manager shape, so the snapshot's family list
/// and the per-table membership lists are empty until a family registry exists.
///
/// # Errors
///
/// Returns [`GameDataCatalogPublishError::Resolve`] when the composed manager
/// shapes and their table inputs do not build a resolvable manager catalog —
/// an unsatisfied dependency, a cycle, or a schema-hash mismatch.
///
/// # Panics
///
/// Panics if a resolved catalog entry has no node in the same resolver's
/// dependency graph; the entries and the graph come from one
/// `build_manager_catalog` result, so they cannot disagree.
#[instrument(skip_all)]
pub fn compose_registered_gamedata_catalog_snapshot(
    registries: &Registries,
    discovered_tables: &[DiscoveredGameDataTable],
    generated_unix_ms: u64,
) -> Result<GameDataCatalogSnapshot, GameDataCatalogPublishError> {
    let no_row_schemas = Registry::<RowSchema>::new();
    let no_managers = Registry::<GameDataManagerShape>::new();
    let row_schemas = registries.get::<RowSchema>().unwrap_or(&no_row_schemas);
    let manager_shapes = registries
        .get::<GameDataManagerShape>()
        .unwrap_or(&no_managers);
    let shapes = manager_shapes.entries().copied().collect::<Vec<_>>();
    let table_inputs = registered_table_inputs(&shapes);

    let tables = discovered_tables
        .iter()
        .map(|table| table_descriptor(table, row_schemas))
        .collect::<Vec<_>>();
    let schema_hashes = table_inputs
        .iter()
        .filter_map(|table| {
            schema_hash_for_row(row_schemas, table.row_name()).map(|hash| {
                ManagerProjectionSource::new(ProviderTarget::table(*table), SchemaHash(hash))
            })
        })
        .collect::<Vec<_>>();

    let resolved = build_manager_catalog(ManagerCatalogInput::new(
        &shapes,
        &table_inputs,
        &[],
        &schema_hashes,
    ))?;
    let nodes_by_id = resolved
        .graph()
        .nodes()
        .iter()
        .map(|node| (node.id(), node.node()))
        .collect::<BTreeMap<_, _>>();
    let dependents = dependents_by_dependency(resolved.entries());
    let owners = owner_index(
        &table_inputs,
        discovered_tables,
        row_schemas,
        manager_shapes,
    );
    let managers = resolved
        .entries()
        .iter()
        .map(|entry| {
            let node = nodes_by_id
                .get(&entry.id())
                .copied()
                .expect("resolved catalog entry must have graph node");
            let direct_dependents = dependents
                .get(&entry.id())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(manager_node_ref)
                .collect::<Vec<_>>();
            manager_entry(entry, node, direct_dependents, &owners, discovered_tables)
        })
        .collect::<Vec<_>>();
    let diagnostics = resolved
        .entries()
        .iter()
        .flat_map(|entry| {
            entry.diagnostics().iter().map(|diagnostic| {
                catalog_diagnostic(
                    *diagnostic,
                    &manager_node_key(entry.id()),
                    entry.id().label(),
                )
            })
        })
        .collect::<Vec<_>>();

    let snapshot = GameDataCatalogSnapshot::new(
        GAMEDATA_CATALOG_SNAPSHOT_VERSION,
        generated_unix_ms,
        tables,
        Vec::new(),
        managers,
        diagnostics,
    );
    info!(
        tables = snapshot.tables.len(),
        families = snapshot.families.len(),
        managers = snapshot.managers.len(),
        diagnostics = snapshot.diagnostics.len(),
        "composed registered GameData catalog"
    );
    Ok(snapshot)
}

fn table_descriptor(
    table: &DiscoveredGameDataTable,
    row_schemas: &Registry<RowSchema>,
) -> GameDataTableDescriptor {
    GameDataTableDescriptor {
        name: table.name.clone(),
        row_type: table.row_type.clone(),
        source_root: table.source_root.clone(),
        source_path: table.source_path.clone(),
        owner: owner_label(&table.owner),
        schema_hash: schema_hash_for_row(row_schemas, &table.row_type),
        document_id: table.document_id.clone(),
        schema_type: schema_type_for_row(row_schemas, &table.row_type),
        category: table_category(&table.source_root, &table.source_path),
        row_count: Some(table.row_count),
        families: Vec::new(),
        source_ref: WorkspaceSourceFileRef {
            source_root_key: table.source_root_key.clone(),
            source_path: table.source_path.clone(),
            schema_type: table.source_schema_type.clone(),
        },
    }
}

fn registered_table_inputs(shapes: &[GameDataManagerShape]) -> Vec<TableInputDescriptor> {
    let mut inputs = BTreeMap::<(&'static str, &'static str), TableInputDescriptor>::new();
    for shape in shapes {
        for input in shape.inputs() {
            match *input {
                GameDataManagerInput::Table(table) => insert_table_input(&mut inputs, table),
                GameDataManagerInput::TableFamily(family) => {
                    for table in family.tables() {
                        insert_table_input(&mut inputs, *table);
                    }
                }
                GameDataManagerInput::Provider(provider) => {
                    insert_provider_target(&mut inputs, provider.target());
                }
                // `Product`, `Manager`, and `SourceSchema` inputs name no
                // table, and the enum is `#[non_exhaustive]`, so anything added
                // later is ignored here until it is handled above.
                _ => {}
            }
        }
        if let Some(target) = shape.provides() {
            insert_provider_target(&mut inputs, target);
        }
    }
    inputs.into_values().collect()
}

fn insert_table_input(
    inputs: &mut BTreeMap<(&'static str, &'static str), TableInputDescriptor>,
    table: TableInputDescriptor,
) {
    inputs.insert((table.table_name(), table.row_name()), table);
}

fn insert_provider_target(
    inputs: &mut BTreeMap<(&'static str, &'static str), TableInputDescriptor>,
    target: ProviderTarget,
) {
    if let ProviderTarget::Table {
        table_name,
        row_name,
    } = target
    {
        insert_table_input(inputs, TableInputDescriptor::new(table_name, row_name));
    }
}

fn table_category(source_root: &str, source_path: &str) -> String {
    let path = normalize_catalog_path(source_path);
    path.strip_prefix("gamedata/")
        .unwrap_or(&path)
        .split('/')
        .find(|segment| !segment.trim().is_empty())
        .map(str::to_string)
        .filter(|category| !category.is_empty())
        .unwrap_or_else(|| {
            let source_root = source_root.trim();
            if source_root.is_empty() {
                "gamedata".to_string()
            } else {
                source_root.to_string()
            }
        })
}

fn normalize_catalog_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_matches('/')
        .split('/')
        .filter(|segment| !segment.trim().is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

fn manager_entry(
    entry: &gamedata::authoring::ManagerCatalogEntry,
    node: ManagerNode,
    dependents: Vec<GameDataManagerNodeRef>,
    owners: &BTreeMap<String, String>,
    discovered_tables: &[DiscoveredGameDataTable],
) -> GameDataManagerCatalogEntry {
    let key = manager_node_key(entry.id());
    let owner = owners.get(&key).cloned().unwrap_or_else(|| owner_label(""));
    match node {
        ManagerNode::Explicit(shape) => {
            let row_type = explicit_shape_row_type(&shape);
            GameDataManagerCatalogEntry {
                key: key.clone(),
                name: shape.name().to_string(),
                owner,
                row_type: row_type.clone(),
                kind: manager_shape_kind_label(shape.kind()).to_string(),
                output_type: row_type,
                read_only: entry.is_read_only(),
                provider_target: shape.provides().map(provider_target),
                key_policy: key_policy(shape.key_policy()),
                duplicate_key_policy: duplicate_key_policy_label(shape.duplicate_key_policy())
                    .to_string(),
                inputs: shape
                    .inputs()
                    .iter()
                    .copied()
                    .map(|input| manager_input(input, discovered_tables))
                    .collect(),
                row_filters: shape
                    .row_filters()
                    .iter()
                    .copied()
                    .map(row_filter)
                    .collect(),
                projection_transforms: shape
                    .projection_transforms()
                    .iter()
                    .copied()
                    .map(projection_transform)
                    .collect(),
                secondary_indexes: shape
                    .secondary_indexes()
                    .iter()
                    .copied()
                    .map(secondary_index)
                    .collect(),
                source_targets: entry
                    .source_targets()
                    .iter()
                    .copied()
                    .map(provider_target)
                    .collect(),
                dependencies: entry
                    .dependencies()
                    .iter()
                    .copied()
                    .map(manager_node_ref)
                    .collect(),
                dependents,
                diagnostics: entry
                    .diagnostics()
                    .iter()
                    .copied()
                    .map(|diagnostic| catalog_diagnostic(diagnostic, &key, shape.name()))
                    .collect(),
                projection_hash: entry.projection_hash().0.to_vec(),
            }
        }
        ManagerNode::DefaultProvider(target) => GameDataManagerCatalogEntry {
            key: key.clone(),
            name: format!("{} Provider", target.name()),
            owner,
            row_type: target.row_name().to_string(),
            kind: "automatic provider".to_string(),
            output_type: target.row_name().to_string(),
            read_only: true,
            provider_target: Some(provider_target(target)),
            key_policy: key_policy(KeyPolicy::row_handle()),
            duplicate_key_policy: duplicate_key_policy_label(DuplicateKeyPolicy::Error).to_string(),
            inputs: vec![provider_input(target, discovered_tables)],
            row_filters: Vec::new(),
            projection_transforms: Vec::new(),
            secondary_indexes: Vec::new(),
            source_targets: entry
                .source_targets()
                .iter()
                .copied()
                .map(provider_target)
                .collect(),
            dependencies: entry
                .dependencies()
                .iter()
                .copied()
                .map(manager_node_ref)
                .collect(),
            dependents,
            diagnostics: entry
                .diagnostics()
                .iter()
                .copied()
                .map(|diagnostic| catalog_diagnostic(diagnostic, &key, target.name()))
                .collect(),
            projection_hash: entry.projection_hash().0.to_vec(),
        },
    }
}

/// Catalog owners, read off composition attribution: the owning gem's id.
fn owner_index(
    table_inputs: &[TableInputDescriptor],
    discovered_tables: &[DiscoveredGameDataTable],
    row_schemas: &Registry<RowSchema>,
    manager_shapes: &Registry<GameDataManagerShape>,
) -> BTreeMap<String, String> {
    let mut owners = BTreeMap::new();
    for table in table_inputs {
        let owner = discovered_table(*table, discovered_tables)
            .map(|source| source.owner.as_str())
            .or_else(|| row_schema_owner(row_schemas, table.row_name()))
            .unwrap_or_default();
        owners.insert(
            manager_node_key(ManagerNodeId::Provider(ProviderTarget::table(*table))),
            owner_label(owner),
        );
    }
    for attributed in manager_shapes {
        owners.insert(
            manager_node_key(ManagerNodeId::Explicit(attributed.entry.name())),
            owner_label(attributed.instance.gem.as_str()),
        );
    }
    owners
}

fn dependents_by_dependency(
    entries: &[gamedata::authoring::ManagerCatalogEntry],
) -> BTreeMap<ManagerNodeId, Vec<ManagerNodeId>> {
    let mut by_dependency: BTreeMap<ManagerNodeId, BTreeSet<ManagerNodeId>> = BTreeMap::new();
    for entry in entries {
        for dependency in entry.dependencies() {
            by_dependency
                .entry(*dependency)
                .or_default()
                .insert(entry.id());
        }
    }
    by_dependency
        .into_iter()
        .map(|(dependency, dependents)| (dependency, dependents.into_iter().collect()))
        .collect()
}

fn explicit_shape_row_type(shape: &gamedata::manager::GameDataManagerShape) -> String {
    if let Some(target) = shape.provides() {
        return target.row_name().to_string();
    }
    for input in shape.inputs() {
        match *input {
            GameDataManagerInput::Table(table) => return table.row_name().to_string(),
            GameDataManagerInput::TableFamily(family) => return family.row_name().to_string(),
            GameDataManagerInput::Product(product) => return product.rust_type().to_string(),
            GameDataManagerInput::SourceSchema(schema) => return schema.schema_type().to_string(),
            // `Manager` and `Provider` inputs carry no row type of their own,
            // and the enum is `#[non_exhaustive]`, so anything added later
            // falls through to the shape name below until it is handled above.
            _ => {}
        }
    }
    shape.name().to_string()
}

fn provider_input(
    target: ProviderTarget,
    discovered_tables: &[DiscoveredGameDataTable],
) -> ProtoManagerInput {
    if let ProviderTarget::Table {
        table_name,
        row_name,
    } = target
    {
        return table_input(
            "table",
            TableInputDescriptor::new(table_name, row_name),
            discovered_tables,
        );
    }
    let provider = provider_target(target);
    ProtoManagerInput {
        kind: provider.kind.clone(),
        name: provider.name,
        row_type: provider.row_type,
        source_root: String::new(),
        source_path: String::new(),
        detail: "synthesized read-only table provider".to_string(),
        provider_kind: "automatic".to_string(),
    }
}

fn manager_input(
    input: GameDataManagerInput,
    discovered_tables: &[DiscoveredGameDataTable],
) -> ProtoManagerInput {
    match input {
        GameDataManagerInput::Table(table) => table_input("table", table, discovered_tables),
        GameDataManagerInput::TableFamily(family) => ProtoManagerInput {
            kind: "family".to_string(),
            name: family.name().to_string(),
            row_type: family.row_name().to_string(),
            source_root: String::new(),
            source_path: String::new(),
            detail: format!("{} tables", family.tables().len()),
            provider_kind: "table family".to_string(),
        },
        GameDataManagerInput::Product(product) => ProtoManagerInput {
            kind: "product".to_string(),
            name: product.asset_path().to_string(),
            row_type: product.rust_type().to_string(),
            source_root: "asset-product".to_string(),
            source_path: product.asset_path().to_string(),
            detail: product_format_label(product.format()).to_string(),
            provider_kind: "product".to_string(),
        },
        GameDataManagerInput::Manager(manager) => ProtoManagerInput {
            kind: "manager".to_string(),
            name: manager.manager().to_string(),
            row_type: String::new(),
            source_root: String::new(),
            source_path: String::new(),
            detail: "manager dependency".to_string(),
            provider_kind: "manager".to_string(),
        },
        GameDataManagerInput::Provider(provider) => {
            let target = provider_target(provider.target());
            ProtoManagerInput {
                kind: "provider".to_string(),
                name: target.name,
                row_type: target.row_type,
                source_root: String::new(),
                source_path: String::new(),
                detail: target.kind,
                provider_kind: "provider".to_string(),
            }
        }
        GameDataManagerInput::SourceSchema(schema) => ProtoManagerInput {
            kind: "source schema".to_string(),
            name: schema.schema_type().to_string(),
            row_type: schema.schema_type().to_string(),
            source_root: String::new(),
            source_path: String::new(),
            detail: source_schema_role_label(schema.role()).to_string(),
            provider_kind: "schema".to_string(),
        },
        _ => ProtoManagerInput {
            kind: "unknown".to_string(),
            name: "unsupported input".to_string(),
            row_type: String::new(),
            source_root: String::new(),
            source_path: String::new(),
            detail: "unsupported descriptor input".to_string(),
            provider_kind: "unknown".to_string(),
        },
    }
}

fn table_input(
    kind: &str,
    table: TableInputDescriptor,
    discovered_tables: &[DiscoveredGameDataTable],
) -> ProtoManagerInput {
    let source = discovered_table(table, discovered_tables);
    let source_root = source.map_or("", |source| source.source_root.as_str());
    let source_path = source.map_or("", |source| source.source_path.as_str());
    ProtoManagerInput {
        kind: kind.to_string(),
        name: table.table_name().to_string(),
        row_type: table.row_name().to_string(),
        source_root: source_root.to_string(),
        source_path: source_path.to_string(),
        detail: if source_root.is_empty() && source_path.is_empty() {
            "discovered table source unavailable".to_string()
        } else {
            format!("{source_root}:{source_path}")
        },
        provider_kind: "table".to_string(),
    }
}

fn discovered_table(
    table: TableInputDescriptor,
    discovered_tables: &[DiscoveredGameDataTable],
) -> Option<&DiscoveredGameDataTable> {
    discovered_tables
        .iter()
        .find(|source| source.name == table.table_name() && source.row_type == table.row_name())
}

fn provider_target(target: ProviderTarget) -> GameDataProviderTarget {
    match target {
        ProviderTarget::Table {
            table_name,
            row_name,
        } => GameDataProviderTarget {
            kind: "table".to_string(),
            name: table_name.to_string(),
            row_type: row_name.to_string(),
        },
        ProviderTarget::Family {
            family_name,
            row_name,
        } => GameDataProviderTarget {
            kind: "family".to_string(),
            name: family_name.to_string(),
            row_type: row_name.to_string(),
        },
    }
}

fn manager_node_ref(id: ManagerNodeId) -> GameDataManagerNodeRef {
    GameDataManagerNodeRef {
        key: manager_node_key(id),
        label: id.label().to_string(),
        kind: match id {
            ManagerNodeId::Explicit(_) => "authored",
            ManagerNodeId::Provider(_) => "provider",
        }
        .to_string(),
    }
}

fn manager_node_key(id: ManagerNodeId) -> String {
    match id {
        ManagerNodeId::Explicit(name) => format!("manager:{name}"),
        ManagerNodeId::Provider(target) => match target {
            ProviderTarget::Table { table_name, .. } => format!("provider:table:{table_name}"),
            ProviderTarget::Family { family_name, .. } => format!("provider:family:{family_name}"),
        },
    }
}

fn key_policy(policy: KeyPolicy) -> GameDataKeyPolicy {
    GameDataKeyPolicy {
        kind: key_kind_label(policy.kind()).to_string(),
        transforms: policy
            .transforms()
            .iter()
            .copied()
            .map(key_transform_label)
            .map(str::to_string)
            .collect(),
        reject_zero_crc: policy.rejects_zero_crc(),
        store_key_text: policy.stores_key_text(),
    }
}

fn row_filter(filter: gamedata::manager::RowFilter) -> GameDataRowFilter {
    GameDataRowFilter {
        field: filter.field().to_string(),
        predicate: row_filter_predicate_label(filter.predicate()).to_string(),
        compare_field: filter.compare_field().unwrap_or_default().to_string(),
    }
}

fn projection_transform(
    transform: gamedata::manager::ProjectionTransform,
) -> GameDataProjectionTransform {
    GameDataProjectionTransform {
        field: transform.field().to_string(),
        source_column: transform.source_column().unwrap_or_default().to_string(),
        kind: projection_transform_label(transform.kind()).to_string(),
    }
}

fn secondary_index(index: gamedata::manager::SecondaryIndexDescriptor) -> GameDataSecondaryIndex {
    GameDataSecondaryIndex {
        name: index.name().to_string(),
        field: index.field().to_string(),
        key_kind: secondary_index_key_kind_label(index.key_kind()).to_string(),
        storage: secondary_index_storage_label(index.storage()).to_string(),
        duplicate_key_policy: duplicate_key_policy_label(index.duplicate_key_policy()).to_string(),
    }
}

fn catalog_diagnostic(
    diagnostic: ManagerCatalogDiagnostic,
    target_key: &str,
    target_label: &str,
) -> GameDataCatalogDiagnostic {
    match diagnostic {
        ManagerCatalogDiagnostic::MissingSchemaHash { target } => GameDataCatalogDiagnostic {
            code: "missing-schema-hash".to_string(),
            message: format!(
                "{} provider `{}` is missing a row schema hash",
                target.kind(),
                target.name()
            ),
            target_key: target_key.to_string(),
            target_label: target_label.to_string(),
        },
        _ => GameDataCatalogDiagnostic {
            code: "unknown".to_string(),
            message: "unrecognized manager catalog diagnostic".to_string(),
            target_key: target_key.to_string(),
            target_label: target_label.to_string(),
        },
    }
}

fn schema_hash_for_row(row_schemas: &Registry<RowSchema>, row_name: &str) -> Option<u64> {
    registered_row_schema(row_schemas, row_name)
        .map(|(descriptor, _owner)| row_schema_hash(descriptor).0)
}

fn schema_type_for_row(row_schemas: &Registry<RowSchema>, row_name: &str) -> String {
    registered_row_schema(row_schemas, row_name).map_or_else(
        || row_name.to_string(),
        |(descriptor, _owner)| descriptor.name().to_string(),
    )
}

fn row_schema_owner(row_schemas: &Registry<RowSchema>, row_name: &str) -> Option<&'static str> {
    registered_row_schema(row_schemas, row_name).map(|(_descriptor, owner)| owner)
}

fn registered_row_schema(
    row_schemas: &Registry<RowSchema>,
    row_name: &str,
) -> Option<(&'static RowSchemaDescriptor, &'static str)> {
    row_schemas
        .iter()
        .map(|attributed| (attributed.entry.descriptor(), attributed.instance.gem))
        .find(|(descriptor, _gem)| row_names_match(descriptor.name(), row_name))
        .map(|(descriptor, gem)| (descriptor, gem.as_str()))
}

fn row_names_match(registered_name: &str, row_name: &str) -> bool {
    registered_name == row_name
        || registered_name.rsplit([':', '.']).next() == Some(row_name)
        || row_name.rsplit([':', '.']).next() == Some(registered_name)
}

fn owner_label(owner: &str) -> String {
    if owner.trim().is_empty() {
        "unowned descriptors".to_string()
    } else {
        owner.to_string()
    }
}

const fn manager_shape_kind_label(kind: ManagerShapeKind) -> &'static str {
    match kind {
        ManagerShapeKind::SingleTableIndex => "single table index",
        ManagerShapeKind::TableFamilyIndex => "table family index",
        ManagerShapeKind::CrcKeyProjection => "CRC key projection",
        ManagerShapeKind::EnumKeyProjection => "enum key projection",
        ManagerShapeKind::NumericKeyProjection => "numeric key projection",
        ManagerShapeKind::StringKeyProjection => "string key projection",
        ManagerShapeKind::PartitionedProjection => "partitioned projection",
        ManagerShapeKind::FallbackProjection => "fallback projection",
        ManagerShapeKind::RowProjection => "row projection",
        ManagerShapeKind::ProductAssetResource => "product asset resource",
        ManagerShapeKind::ComposedResource => "composed resource",
        ManagerShapeKind::Custom(value) => value,
        _ => "unknown",
    }
}

const fn duplicate_key_policy_label(policy: DuplicateKeyPolicy) -> &'static str {
    match policy {
        DuplicateKeyPolicy::Error => "error",
        DuplicateKeyPolicy::FirstWins => "first wins",
        DuplicateKeyPolicy::Overwrite => "overwrite",
        DuplicateKeyPolicy::Multi => "multi",
        _ => "unknown",
    }
}

const fn key_kind_label(kind: KeyKind) -> &'static str {
    match kind {
        KeyKind::RowHandle => "row handle",
        KeyKind::Crc32 => "CRC32",
        KeyKind::Enum => "enum",
        KeyKind::Numeric => "numeric",
        KeyKind::String => "string",
        KeyKind::Custom(value) => value,
        _ => "unknown",
    }
}

const fn key_transform_label(transform: KeyTransform) -> &'static str {
    match transform {
        KeyTransform::Trim => "trim",
        KeyTransform::Lowercase => "lowercase",
        KeyTransform::RemoveSpaceAndTab => "remove space/tab",
        KeyTransform::Crc32 => "CRC32",
        KeyTransform::Custom(value) => value,
        _ => "unknown",
    }
}

const fn product_format_label(format: ProductInputFormat) -> &'static str {
    match format {
        ProductInputFormat::ObjectStream => "object stream",
        ProductInputFormat::Xml => "XML",
        ProductInputFormat::Binary => "binary",
        ProductInputFormat::Other(value) => value,
        _ => "unknown",
    }
}

const fn source_schema_role_label(role: SourceSchemaRole) -> &'static str {
    match role {
        SourceSchemaRole::Prefab => "prefab",
        SourceSchemaRole::StaticData => "static data",
        SourceSchemaRole::Config => "config",
        SourceSchemaRole::Other(value) => value,
        _ => "unknown",
    }
}

const fn row_filter_predicate_label(predicate: RowFilterPredicate) -> &'static str {
    match predicate {
        RowFilterPredicate::FieldFalse => "field false",
        RowFilterPredicate::FieldTrue => "field true",
        RowFilterPredicate::BoolTrueWhenPresent => "bool true when present",
        RowFilterPredicate::F32GreaterThanOrEqualZero => "f32 >= 0",
        RowFilterPredicate::F32LessThanOrEqualZero => "f32 <= 0",
        RowFilterPredicate::F32AnyGreaterThanZero => "any f32 > 0",
        RowFilterPredicate::I32LessThanOrEqualZero => "i32 <= 0",
        RowFilterPredicate::LowercaseCrcStringNonZero => "lowercase CRC string non-zero",
        RowFilterPredicate::StringNotEqualToField => "string not equal to field",
        RowFilterPredicate::Custom(value) => value,
        _ => "unknown",
    }
}

const fn projection_transform_label(kind: ProjectionTransformKind) -> &'static str {
    match kind {
        ProjectionTransformKind::Identity => "identity",
        ProjectionTransformKind::PlusJoinedList => "plus-joined list",
        ProjectionTransformKind::NonEmptyString => "non-empty string",
        ProjectionTransformKind::OptionalFirstString => "optional first string",
        ProjectionTransformKind::Crc32 => "CRC32",
        ProjectionTransformKind::LowercaseCrcString => "lowercase CRC string",
        ProjectionTransformKind::CrcList => "CRC list",
        ProjectionTransformKind::TrimmedLowercaseCrcStringList => {
            "trimmed lowercase CRC string list"
        }
        ProjectionTransformKind::ForeignKeyTargetKey => "foreign key target key",
        ProjectionTransformKind::ForeignKeyTargetLowercaseCrc => "foreign key target lowercase CRC",
        ProjectionTransformKind::ForeignKeyRow => "foreign key row",
        ProjectionTransformKind::ForeignKeyRowList => "foreign key row list",
        ProjectionTransformKind::F32RangeInclusive => "f32 inclusive range",
        ProjectionTransformKind::F32MinutesToSeconds => "f32 minutes to seconds",
        ProjectionTransformKind::U32ToU16BelowMax => "u32 to u16 below max",
        ProjectionTransformKind::Custom(value) => value,
        _ => "unknown",
    }
}

const fn secondary_index_key_kind_label(kind: SecondaryIndexKeyKind) -> &'static str {
    match kind {
        SecondaryIndexKeyKind::U16 => "u16",
        SecondaryIndexKeyKind::NonZeroU32 => "non-zero u32",
        SecondaryIndexKeyKind::Crc32 => "CRC32",
        SecondaryIndexKeyKind::Custom(value) => value,
        _ => "unknown",
    }
}

const fn secondary_index_storage_label(storage: SecondaryIndexStorage) -> &'static str {
    match storage {
        SecondaryIndexStorage::HashMap => "hash map",
        SecondaryIndexStorage::SparseVec => "sparse vec",
        SecondaryIndexStorage::DescendingF32 => "descending f32",
        SecondaryIndexStorage::Custom(value) => value,
        _ => "unknown",
    }
}

fn now_unix_ms() -> Result<u64, GameDataCatalogPublishError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| GameDataCatalogPublishError::ClockBeforeUnixEpoch)?;
    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use az_gem_contract::{
        Composer, Contribution, ContributionDescriptor, ContributionId, GemContext, GemId,
        GemTargetRole, ProductActivation, declare_caps,
    };
    use az_proto_project::load_gamedata_catalog_side_channel;
    use gamedata::{CellType, ColumnSchemaDescriptor, RowSchemaCatalog, ScalarType};

    const ITEM_COLUMNS: &[ColumnSchemaDescriptor] =
        &[
            ColumnSchemaDescriptor::new("item_id", "ItemID", CellType::Scalar(ScalarType::RowKey))
                .key_candidate(true),
        ];
    const ITEM_ROW: RowSchemaDescriptor = RowSchemaDescriptor::new("game::ItemRow", ITEM_COLUMNS);
    const ITEM_CATALOG: RowSchemaCatalog = RowSchemaCatalog::new(&[ITEM_ROW]);
    const ITEMS: TableInputDescriptor = TableInputDescriptor::new("Items", "game::ItemRow");
    const MANAGER_INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Table(ITEMS)];
    const MANAGER: GameDataManagerShape = GameDataManagerShape::new(
        "ItemManager",
        ManagerShapeKind::SingleTableIndex,
        MANAGER_INPUTS,
    );

    declare_caps!(CatalogCaps:);

    /// Stands in for a project gem's `GameData` contribution.
    struct Schema;

    impl Contribution for Schema {
        type Caps = CatalogCaps;

        fn descriptor(&self) -> ContributionDescriptor {
            ContributionDescriptor {
                gem: GemId::new("azoth.project-host-tests"),
                contribution: ContributionId::new("gamedata"),
                roles: &[],
            }
        }

        fn register(&self, ctx: &mut GemContext<'_, CatalogCaps>) {
            ctx.registrar::<RowSchema>()
                .register_many(ITEM_CATALOG.rows());
            ctx.registrar::<GameDataManagerShape>().register(MANAGER);
        }
    }

    fn composed() -> Composer {
        let mut composer = Composer::new(GemTargetRole::ProjectHost);
        crate::tests::floor(&mut composer);
        composer
            .add(Schema, ProductActivation::default())
            .expect("gamedata composes unconditionally");
        composer
    }

    fn discovered_items() -> DiscoveredGameDataTable {
        DiscoveredGameDataTable {
            name: "Items".to_string(),
            row_type: "game::ItemRow".to_string(),
            source_root: "project:test:assets".to_string(),
            source_root_key: "project:test:assets".to_string(),
            source_schema_type: "azoth.gamedata.TableSource".to_string(),
            source_path: "gamedata/items/master.ron".to_string(),
            document_id: "gamedata/items/master.ron".to_string(),
            owner: "azoth.project-host-tests".to_string(),
            row_count: 17,
        }
    }

    #[test]
    fn table_descriptor_derives_editor_lookup_fields_from_composed_row_schemas() {
        let composer = composed();
        let row_schemas = composer
            .registries()
            .get::<RowSchema>()
            .expect("row schemas were composed");
        let table = discovered_items();

        let descriptor = table_descriptor(&table, row_schemas);

        assert_eq!(descriptor.name, "Items");
        assert_eq!(descriptor.row_type, "game::ItemRow");
        assert_eq!(descriptor.source_root, "project:test:assets");
        assert_eq!(descriptor.source_path, "gamedata/items/master.ron");
        assert_eq!(descriptor.document_id, "gamedata/items/master.ron");
        assert_eq!(descriptor.schema_type, "game::ItemRow");
        assert_eq!(descriptor.source_ref.source_root_key, "project:test:assets");
        assert_eq!(
            descriptor.source_ref.schema_type,
            "azoth.gamedata.TableSource"
        );
        assert_eq!(descriptor.category, "items");
        assert!(descriptor.schema_hash.is_some());
        assert_eq!(descriptor.row_count, Some(17));
        assert!(
            descriptor.families.is_empty(),
            "table families have no registry to compose from"
        );
    }

    #[test]
    fn composed_managers_are_owned_by_the_contributing_gem() {
        let composer = composed();
        let snapshot = compose_registered_gamedata_catalog_snapshot(
            composer.registries(),
            &[discovered_items()],
            7,
        )
        .expect("composed GameData catalog");

        let manager = snapshot
            .managers
            .iter()
            .find(|entry| entry.name == "ItemManager")
            .expect("composed manager shape reaches the catalog");
        assert_eq!(manager.owner, "azoth.project-host-tests");
        assert!(snapshot.families.is_empty());
        assert_eq!(snapshot.tables.len(), 1);
    }

    #[test]
    fn an_uncomposed_registry_set_yields_no_managers() {
        let registries = Registries::new();
        let snapshot =
            compose_registered_gamedata_catalog_snapshot(&registries, &[discovered_items()], 7)
                .expect("an empty composition is still a valid catalog");

        assert!(snapshot.managers.is_empty());
        assert_eq!(snapshot.tables.len(), 1);
    }

    #[test]
    fn gamedata_catalog_side_channel_round_trips_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let side_channel = GameDataCatalogSideChannel::new(temp.path());
        let snapshot = GameDataCatalogSnapshot::new(
            GAMEDATA_CATALOG_SNAPSHOT_VERSION,
            123,
            vec![GameDataTableDescriptor {
                name: "Items".to_string(),
                row_type: "ItemRow".to_string(),
                source_root: "gamedata".to_string(),
                source_path: "items.ron".to_string(),
                owner: "test-gem".to_string(),
                schema_hash: Some(42),
                document_id: "gamedata/items.ron".to_string(),
                schema_type: "ItemRow".to_string(),
                category: "items".to_string(),
                row_count: None,
                families: Vec::new(),
                source_ref: WorkspaceSourceFileRef {
                    source_root_key: "project:test:assets".to_string(),
                    source_path: "items.ron".to_string(),
                    schema_type: "azoth.gamedata.TableSource".to_string(),
                },
            }],
            Vec::new(),
            vec![GameDataManagerCatalogEntry {
                key: "provider:table:Items".to_string(),
                name: "Items Provider".to_string(),
                owner: "test-gem".to_string(),
                row_type: "ItemRow".to_string(),
                kind: "automatic provider".to_string(),
                output_type: "ItemRow".to_string(),
                read_only: true,
                provider_target: Some(GameDataProviderTarget {
                    kind: "table".to_string(),
                    name: "Items".to_string(),
                    row_type: "ItemRow".to_string(),
                }),
                key_policy: GameDataKeyPolicy {
                    kind: "row handle".to_string(),
                    transforms: Vec::new(),
                    reject_zero_crc: false,
                    store_key_text: false,
                },
                duplicate_key_policy: "error".to_string(),
                inputs: vec![ProtoManagerInput {
                    kind: "table".to_string(),
                    name: "Items".to_string(),
                    row_type: "ItemRow".to_string(),
                    source_root: "gamedata".to_string(),
                    source_path: "items.ron".to_string(),
                    detail: "gamedata:items.ron".to_string(),
                    provider_kind: "table".to_string(),
                }],
                row_filters: Vec::new(),
                projection_transforms: Vec::new(),
                secondary_indexes: Vec::new(),
                source_targets: vec![GameDataProviderTarget {
                    kind: "table".to_string(),
                    name: "Items".to_string(),
                    row_type: "ItemRow".to_string(),
                }],
                dependencies: Vec::new(),
                dependents: Vec::new(),
                diagnostics: Vec::new(),
                projection_hash: vec![7; 32],
            }],
            Vec::new(),
        );

        let written = side_channel.write_snapshot(&snapshot).unwrap();
        let decoded = load_gamedata_catalog_side_channel(&written.handle).unwrap();

        assert_eq!(decoded, snapshot);
    }
}
