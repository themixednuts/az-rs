//! Resolved `GameData` manager catalog for editor and build planning.
//!
//! This layer combines table/family descriptors, explicit manager shapes, and
//! synthesized read-only provider nodes into one deterministic projection
//! catalog. It is IO-free so project-host, asset builders, and tests can build
//! the same catalog from their current descriptor inventory.

use std::collections::{BTreeMap, BTreeSet};

use crate::release::{ProjectionHash, SchemaHash};

use super::manager_fingerprint::{
    ManagerProjectionDependency, ManagerProjectionSource, manager_projection_fingerprint_with_deps,
};
use super::manager_graph::{
    ManagerGraphError, ManagerNode, ManagerNodeId, ResolvedManagerGraph, build_manager_graph,
};
use crate::manager::{
    DuplicateKeyPolicy, GameDataManagerInput, GameDataManagerShape, ProviderTarget,
    TableFamilyDescriptor, TableInputDescriptor,
};

/// Input descriptors for building a resolved manager catalog.
#[derive(Debug, Clone, Copy)]
pub struct ManagerCatalogInput<'a> {
    shapes: &'a [GameDataManagerShape],
    tables: &'a [TableInputDescriptor],
    families: &'a [TableFamilyDescriptor],
    schema_hashes: &'a [ManagerProjectionSource],
}

impl<'a> ManagerCatalogInput<'a> {
    /// Creates a catalog input from static/project descriptor slices.
    #[inline]
    #[must_use]
    pub const fn new(
        shapes: &'a [GameDataManagerShape],
        tables: &'a [TableInputDescriptor],
        families: &'a [TableFamilyDescriptor],
        schema_hashes: &'a [ManagerProjectionSource],
    ) -> Self {
        Self {
            shapes,
            tables,
            families,
            schema_hashes,
        }
    }

    /// Explicit manager shapes.
    #[inline]
    #[must_use]
    pub const fn shapes(self) -> &'a [GameDataManagerShape] {
        self.shapes
    }

    /// Concrete table descriptors.
    #[inline]
    #[must_use]
    pub const fn tables(self) -> &'a [TableInputDescriptor] {
        self.tables
    }

    /// Table-family descriptors.
    #[inline]
    #[must_use]
    pub const fn families(self) -> &'a [TableFamilyDescriptor] {
        self.families
    }

    /// Schema hashes for direct table/family provider targets.
    #[inline]
    #[must_use]
    pub const fn schema_hashes(self) -> &'a [ManagerProjectionSource] {
        self.schema_hashes
    }
}

/// Resolved catalog entry for one manager/provider node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerCatalogEntry {
    id: ManagerNodeId,
    read_only: bool,
    projection_hash: ProjectionHash,
    source_targets: Vec<ProviderTarget>,
    dependencies: Vec<ManagerNodeId>,
    diagnostics: Vec<ManagerCatalogDiagnostic>,
}

impl ManagerCatalogEntry {
    /// Resolved node identity.
    #[inline]
    #[must_use]
    pub const fn id(&self) -> ManagerNodeId {
        self.id
    }

    /// Whether this entry is a synthesized read-only provider.
    #[inline]
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.read_only
    }

    /// Stable projection fingerprint for this resolved node.
    #[inline]
    #[must_use]
    pub const fn projection_hash(&self) -> ProjectionHash {
        self.projection_hash
    }

    /// Direct table/family targets read by this node.
    #[inline]
    #[must_use]
    pub fn source_targets(&self) -> &[ProviderTarget] {
        &self.source_targets
    }

    /// Resolved manager/provider dependencies.
    #[inline]
    #[must_use]
    pub fn dependencies(&self) -> &[ManagerNodeId] {
        &self.dependencies
    }

    /// Non-fatal catalog diagnostics attached to this entry.
    #[inline]
    #[must_use]
    pub fn diagnostics(&self) -> &[ManagerCatalogDiagnostic] {
        &self.diagnostics
    }
}

/// Non-fatal manager catalog diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagerCatalogDiagnostic {
    /// A direct table/family input has no available schema hash.
    MissingSchemaHash { target: ProviderTarget },
}

/// Impact of deleting a table or table-family provider target from the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagerDeleteImpact {
    target: ProviderTarget,
    affected_targets: Vec<ProviderTarget>,
    broken: Vec<ManagerNodeId>,
    invalidated: Vec<ManagerNodeId>,
}

impl ManagerDeleteImpact {
    /// Provider target requested for deletion.
    #[inline]
    #[must_use]
    pub const fn target(&self) -> ProviderTarget {
        self.target
    }

    /// Concrete targets affected by the deletion. A table deletion also affects
    /// any family provider target that contains the table.
    #[inline]
    #[must_use]
    pub fn affected_targets(&self) -> &[ProviderTarget] {
        &self.affected_targets
    }

    /// Nodes that would become semantically invalid after the deletion.
    #[inline]
    #[must_use]
    pub fn broken(&self) -> &[ManagerNodeId] {
        &self.broken
    }

    /// Transitive dependents that must be rebuilt after affected/broken nodes.
    #[inline]
    #[must_use]
    pub fn invalidated(&self) -> &[ManagerNodeId] {
        &self.invalidated
    }
}

/// Resolved manager catalog in dependency-first order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedManagerCatalog {
    graph: ResolvedManagerGraph,
    entries: Vec<ManagerCatalogEntry>,
    indexes_by_id: BTreeMap<ManagerNodeId, usize>,
    dependents_by_dependency: BTreeMap<ManagerNodeId, Vec<ManagerNodeId>>,
    targets_by_table_name: BTreeMap<&'static str, Vec<ProviderTarget>>,
}

impl ResolvedManagerCatalog {
    /// Resolved dependency graph used to build this catalog.
    #[inline]
    #[must_use]
    pub const fn graph(&self) -> &ResolvedManagerGraph {
        &self.graph
    }

    /// Catalog entries in dependency-first order.
    #[inline]
    #[must_use]
    pub fn entries(&self) -> &[ManagerCatalogEntry] {
        &self.entries
    }

    /// Finds one catalog entry by resolved node id.
    #[must_use]
    pub fn entry(&self, id: ManagerNodeId) -> Option<&ManagerCatalogEntry> {
        self.indexes_by_id
            .get(&id)
            .and_then(|index| self.entries.get(*index))
    }

    /// Direct dependents of the provider for a table or table family.
    #[must_use]
    pub fn provider_dependents(&self, target: ProviderTarget) -> Vec<ManagerNodeId> {
        self.dependents_by_dependency
            .get(&ManagerNodeId::Provider(target))
            .cloned()
            .unwrap_or_default()
    }

    /// Computes manager impact for deleting a table or table-family provider
    /// target from the catalog.
    #[must_use]
    pub fn delete_impact(&self, target: ProviderTarget) -> ManagerDeleteImpact {
        let affected_targets = self.affected_targets_for_delete(target);
        let affected_target_set = affected_targets.iter().copied().collect::<BTreeSet<_>>();
        let mut broken = BTreeSet::new();

        for entry in &self.entries {
            if entry
                .source_targets()
                .iter()
                .any(|source| affected_target_set.contains(source))
                || matches!(
                    entry.id(),
                    ManagerNodeId::Provider(provider) if affected_target_set.contains(&provider)
                )
                || entry.dependencies().iter().any(|dependency| {
                    matches!(
                        dependency,
                        ManagerNodeId::Provider(provider) if affected_target_set.contains(provider)
                    )
                })
            {
                broken.insert(entry.id());
            }
        }

        let mut invalidated = BTreeSet::new();
        let mut stack = broken.iter().copied().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if let Some(dependents) = self.dependents_by_dependency.get(&id) {
                for dependent in dependents {
                    if !broken.contains(dependent) && invalidated.insert(*dependent) {
                        stack.push(*dependent);
                    }
                }
            }
        }

        ManagerDeleteImpact {
            target,
            affected_targets,
            broken: self.ordered_ids(&broken),
            invalidated: self.ordered_ids(&invalidated),
        }
    }

    fn affected_targets_for_delete(&self, target: ProviderTarget) -> Vec<ProviderTarget> {
        let mut targets = BTreeSet::new();
        targets.insert(target);
        if let ProviderTarget::Table { table_name, .. } = target
            && let Some(related) = self.targets_by_table_name.get(table_name)
        {
            targets.extend(related.iter().copied());
        }
        targets.into_iter().collect()
    }

    fn ordered_ids(&self, ids: &BTreeSet<ManagerNodeId>) -> Vec<ManagerNodeId> {
        self.entries
            .iter()
            .map(ManagerCatalogEntry::id)
            .filter(|id| ids.contains(id))
            .collect()
    }
}

/// Builds a resolved manager catalog with dependency-aware projection hashes.
///
/// # Errors
///
/// Returns graph-level errors for invalid manager descriptors, duplicate names,
/// duplicate provider claims, dangling targets, or dependency cycles.
///
/// # Panics
///
/// Panics if the graph's own topological order names a node the graph does not
/// hold, which would mean `build_manager_graph` returned an inconsistent graph.
pub fn build_manager_catalog(
    input: ManagerCatalogInput<'_>,
) -> Result<ResolvedManagerCatalog, ManagerGraphError> {
    let graph = build_manager_graph(input.shapes(), input.tables(), input.families())?;
    let schema_hashes_by_target = schema_hashes_by_target(input.schema_hashes(), input.families());
    let mut projection_hashes = BTreeMap::new();
    let mut entries = Vec::with_capacity(graph.nodes().len());

    for id in graph.topological_order() {
        let node = graph.node(*id).expect("topological node must exist");
        let source_targets = direct_source_targets(node.node());
        let mut diagnostics = Vec::new();
        let mut sources = Vec::with_capacity(source_targets.len());
        for target in &source_targets {
            if let Some(schema_hash) = schema_hashes_by_target.get(target).copied() {
                sources.push(ManagerProjectionSource::new(*target, schema_hash));
            } else {
                diagnostics.push(ManagerCatalogDiagnostic::MissingSchemaHash { target: *target });
            }
        }

        let mut dependencies = Vec::with_capacity(node.dependencies().len());
        for dependency in node.dependencies() {
            let dependency_hash = projection_hashes
                .get(dependency)
                .copied()
                .expect("manager graph topological order must be dependency-first");
            dependencies.push(ManagerProjectionDependency::new(
                *dependency,
                dependency_hash,
            ));
        }

        let projection_hash =
            manager_projection_fingerprint_with_deps(node.node(), &sources, &dependencies);
        projection_hashes.insert(*id, projection_hash);
        entries.push(ManagerCatalogEntry {
            id: *id,
            read_only: matches!(node.node(), ManagerNode::DefaultProvider(_)),
            projection_hash,
            source_targets,
            dependencies: node.dependencies().to_vec(),
            diagnostics,
        });
    }

    let indexes_by_id = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.id, index))
        .collect();
    let dependents_by_dependency = dependents_by_dependency(&entries);
    let targets_by_table_name = targets_by_table_name(input.tables(), input.families());

    Ok(ResolvedManagerCatalog {
        graph,
        entries,
        indexes_by_id,
        dependents_by_dependency,
        targets_by_table_name,
    })
}

fn schema_hashes_by_target(
    sources: &[ManagerProjectionSource],
    families: &[TableFamilyDescriptor],
) -> BTreeMap<ProviderTarget, SchemaHash> {
    let mut hashes = sources
        .iter()
        .map(|source| (source.target(), source.schema_hash()))
        .collect::<BTreeMap<_, _>>();
    for family in families {
        let family_target = ProviderTarget::family(*family);
        if !hashes.contains_key(&family_target)
            && let Some(schema_hash) = table_family_schema_hash(*family, &hashes)
        {
            hashes.insert(family_target, schema_hash);
        }
    }
    hashes
}

fn table_family_schema_hash(
    family: TableFamilyDescriptor,
    schema_hashes: &BTreeMap<ProviderTarget, SchemaHash>,
) -> Option<SchemaHash> {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"azoth.gamedata.table-family.schema.v1\0");
    hash_str(&mut hasher, family.name());
    hash_str(&mut hasher, family.row_name());
    hash_duplicate_key_policy(&mut hasher, family.duplicate_key_policy());
    hasher.update(&(family.tables().len() as u64).to_le_bytes());
    for table in family.tables() {
        let target = ProviderTarget::table(*table);
        let schema_hash = schema_hashes.get(&target).copied()?;
        hash_provider_target(&mut hasher, target);
        hasher.update(&schema_hash.0.to_le_bytes());
    }
    let hash = hasher.finalize();
    Some(SchemaHash(u64::from_le_bytes(
        hash.as_bytes()[..8]
            .try_into()
            .expect("schema hash prefix length"),
    )))
}

fn hash_provider_target(hasher: &mut blake3::Hasher, target: ProviderTarget) {
    match target {
        ProviderTarget::Table {
            table_name,
            row_name,
        } => {
            hasher.update(&[1]);
            hash_str(hasher, table_name);
            hash_str(hasher, row_name);
        }
        ProviderTarget::Family {
            family_name,
            row_name,
        } => {
            hasher.update(&[2]);
            hash_str(hasher, family_name);
            hash_str(hasher, row_name);
        }
    }
}

fn hash_duplicate_key_policy(hasher: &mut blake3::Hasher, policy: DuplicateKeyPolicy) {
    match policy {
        DuplicateKeyPolicy::Error => hasher.update(&[1]),
        DuplicateKeyPolicy::FirstWins => hasher.update(&[2]),
        DuplicateKeyPolicy::Overwrite => hasher.update(&[3]),
        DuplicateKeyPolicy::Multi => hasher.update(&[4]),
    };
}

fn hash_str(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn direct_source_targets(node: ManagerNode) -> Vec<ProviderTarget> {
    let mut targets = BTreeSet::new();
    match node {
        ManagerNode::Explicit(shape) => {
            for input in shape.inputs() {
                match *input {
                    GameDataManagerInput::Table(table) => {
                        targets.insert(ProviderTarget::table(table));
                    }
                    GameDataManagerInput::TableFamily(family) => {
                        targets.insert(ProviderTarget::family(family));
                    }
                    GameDataManagerInput::Product(_)
                    | GameDataManagerInput::Manager(_)
                    | GameDataManagerInput::Provider(_)
                    | GameDataManagerInput::SourceSchema(_) => {}
                }
            }
        }
        ManagerNode::DefaultProvider(target) => {
            targets.insert(target);
        }
    }
    targets.into_iter().collect()
}

fn dependents_by_dependency(
    entries: &[ManagerCatalogEntry],
) -> BTreeMap<ManagerNodeId, Vec<ManagerNodeId>> {
    let mut dependents: BTreeMap<ManagerNodeId, BTreeSet<ManagerNodeId>> = BTreeMap::new();
    for entry in entries {
        for dependency in entry.dependencies() {
            dependents
                .entry(*dependency)
                .or_default()
                .insert(entry.id());
        }
    }
    dependents
        .into_iter()
        .map(|(dependency, ids)| (dependency, ids.into_iter().collect()))
        .collect()
}

fn targets_by_table_name(
    tables: &[TableInputDescriptor],
    families: &[TableFamilyDescriptor],
) -> BTreeMap<&'static str, Vec<ProviderTarget>> {
    let mut targets: BTreeMap<&'static str, BTreeSet<ProviderTarget>> = BTreeMap::new();
    for table in tables {
        targets
            .entry(table.table_name())
            .or_default()
            .insert(ProviderTarget::table(*table));
    }
    for family in families {
        let target = ProviderTarget::family(*family);
        for table in family.tables() {
            targets
                .entry(table.table_name())
                .or_default()
                .insert(target);
        }
    }
    targets
        .into_iter()
        .map(|(table, targets)| (table, targets.into_iter().collect()))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::manager::{
        DuplicateKeyPolicy, KeyKind, KeyPolicy, ManagerDependencyDescriptor, ManagerShapeKind,
        ProviderDependencyDescriptor,
    };
    use crate::release::SchemaHash;

    use super::*;

    const ITEM_TABLE: TableInputDescriptor = TableInputDescriptor::new("ItemTable", "ItemRow");
    const NPC_TABLE: TableInputDescriptor = TableInputDescriptor::new("NpcTable", "NpcRow");
    const MTX_ITEM_TABLE: TableInputDescriptor =
        TableInputDescriptor::new("MtxItemTable", "ItemRow");
    const TABLES: &[TableInputDescriptor] = &[ITEM_TABLE, NPC_TABLE, MTX_ITEM_TABLE];
    const ITEM_TARGET: ProviderTarget = ProviderTarget::table(ITEM_TABLE);
    const NPC_TARGET: ProviderTarget = ProviderTarget::table(NPC_TABLE);
    const ITEM_FAMILY_TABLES: &[TableInputDescriptor] = &[ITEM_TABLE, MTX_ITEM_TABLE];
    const ITEM_FAMILY: TableFamilyDescriptor =
        TableFamilyDescriptor::new("ItemFamily", "ItemRow", ITEM_FAMILY_TABLES)
            .with_duplicate_key_policy(DuplicateKeyPolicy::Overwrite);
    const ITEM_FAMILY_TARGET: ProviderTarget = ProviderTarget::family(ITEM_FAMILY);
    const FAMILIES: &[TableFamilyDescriptor] = &[ITEM_FAMILY];
    const ITEM_SCHEMA: ManagerProjectionSource =
        ManagerProjectionSource::new(ITEM_TARGET, SchemaHash(1));
    const ITEM_SCHEMA_CHANGED: ManagerProjectionSource =
        ManagerProjectionSource::new(ITEM_TARGET, SchemaHash(2));
    const NPC_SCHEMA: ManagerProjectionSource =
        ManagerProjectionSource::new(NPC_TARGET, SchemaHash(3));
    const MTX_ITEM_TARGET: ProviderTarget = ProviderTarget::table(MTX_ITEM_TABLE);
    const MTX_ITEM_SCHEMA: ManagerProjectionSource =
        ManagerProjectionSource::new(MTX_ITEM_TARGET, SchemaHash(4));
    const MTX_ITEM_SCHEMA_CHANGED: ManagerProjectionSource =
        ManagerProjectionSource::new(MTX_ITEM_TARGET, SchemaHash(5));

    const DEPENDS_ON_ITEM_PROVIDER_INPUTS: &[GameDataManagerInput] =
        &[GameDataManagerInput::Provider(
            ProviderDependencyDescriptor::new(ITEM_TARGET),
        )];
    const DEPENDS_ON_ITEM_PROVIDER: GameDataManagerShape = GameDataManagerShape::new(
        "DependentManager",
        ManagerShapeKind::ComposedResource,
        DEPENDS_ON_ITEM_PROVIDER_INPUTS,
    );

    const ITEM_PROVIDER_INPUTS: &[GameDataManagerInput] =
        &[GameDataManagerInput::Table(ITEM_TABLE)];
    const ITEM_PROVIDER: GameDataManagerShape = GameDataManagerShape::new(
        "ItemDataManager",
        ManagerShapeKind::SingleTableIndex,
        ITEM_PROVIDER_INPUTS,
    )
    .with_provides(ITEM_TARGET);
    const ITEM_PROVIDER_CRC: GameDataManagerShape = GameDataManagerShape::new(
        "ItemDataManager",
        ManagerShapeKind::SingleTableIndex,
        ITEM_PROVIDER_INPUTS,
    )
    .with_provides(ITEM_TARGET)
    .with_key_policy(KeyPolicy::new(KeyKind::Crc32));

    #[test]
    fn synthesized_provider_is_read_only_and_schema_sensitive() {
        let first = build_manager_catalog(ManagerCatalogInput::new(
            &[DEPENDS_ON_ITEM_PROVIDER],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA],
        ))
        .expect("catalog should resolve");
        let second = build_manager_catalog(ManagerCatalogInput::new(
            &[DEPENDS_ON_ITEM_PROVIDER],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA_CHANGED],
        ))
        .expect("catalog should resolve");
        let provider_id = ManagerNodeId::Provider(ITEM_TARGET);
        let dependent_id = ManagerNodeId::Explicit("DependentManager");

        let provider = first.entry(provider_id).expect("default provider");
        assert!(provider.is_read_only());
        assert_eq!(provider.source_targets(), &[ITEM_TARGET]);
        assert!(provider.diagnostics().is_empty());
        assert_ne!(
            first.entry(dependent_id).unwrap().projection_hash(),
            second.entry(dependent_id).unwrap().projection_hash(),
            "dependent manager must dirty when upstream provider schema changes"
        );
    }

    #[test]
    fn explicit_provider_policy_change_dirties_dependents() {
        let row_handle = build_manager_catalog(ManagerCatalogInput::new(
            &[ITEM_PROVIDER, DEPENDS_ON_ITEM_PROVIDER],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA],
        ))
        .expect("catalog should resolve");
        let crc = build_manager_catalog(ManagerCatalogInput::new(
            &[ITEM_PROVIDER_CRC, DEPENDS_ON_ITEM_PROVIDER],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA],
        ))
        .expect("catalog should resolve");
        let dependent_id = ManagerNodeId::Explicit("DependentManager");

        assert_ne!(
            row_handle.entry(dependent_id).unwrap().projection_hash(),
            crc.entry(dependent_id).unwrap().projection_hash(),
            "dependent manager must dirty when explicit provider output policy changes"
        );
    }

    #[test]
    fn missing_schema_hash_is_entry_diagnostic_not_graph_error() {
        let catalog = build_manager_catalog(ManagerCatalogInput::new(
            &[DEPENDS_ON_ITEM_PROVIDER],
            TABLES,
            FAMILIES,
            &[],
        ))
        .expect("missing schema hash should not block catalog topology");

        assert_eq!(
            catalog
                .entry(ManagerNodeId::Provider(ITEM_TARGET))
                .unwrap()
                .diagnostics(),
            &[ManagerCatalogDiagnostic::MissingSchemaHash {
                target: ITEM_TARGET
            }]
        );
    }

    #[test]
    fn family_schema_hash_is_derived_from_member_table_hashes() {
        const FAMILY_INPUTS: &[GameDataManagerInput] =
            &[GameDataManagerInput::TableFamily(ITEM_FAMILY)];
        const FAMILY_READER: GameDataManagerShape = GameDataManagerShape::new(
            "FamilyReader",
            ManagerShapeKind::TableFamilyIndex,
            FAMILY_INPUTS,
        );

        let first = build_manager_catalog(ManagerCatalogInput::new(
            &[FAMILY_READER],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA, MTX_ITEM_SCHEMA],
        ))
        .expect("catalog should resolve");
        let second = build_manager_catalog(ManagerCatalogInput::new(
            &[FAMILY_READER],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA, MTX_ITEM_SCHEMA_CHANGED],
        ))
        .expect("catalog should resolve");
        let family_reader_id = ManagerNodeId::Explicit("FamilyReader");

        assert!(
            first
                .entry(family_reader_id)
                .unwrap()
                .diagnostics()
                .is_empty()
        );
        assert_ne!(
            first.entry(family_reader_id).unwrap().projection_hash(),
            second.entry(family_reader_id).unwrap().projection_hash(),
            "family reader must dirty when any member table schema changes"
        );
    }

    #[test]
    fn delete_impact_includes_family_targets_and_transitive_dependents() {
        const FAMILY_INPUTS: &[GameDataManagerInput] =
            &[GameDataManagerInput::TableFamily(ITEM_FAMILY)];
        const FAMILY_READER: GameDataManagerShape = GameDataManagerShape::new(
            "FamilyReader",
            ManagerShapeKind::TableFamilyIndex,
            FAMILY_INPUTS,
        );
        const DOWNSTREAM_INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Manager(
            ManagerDependencyDescriptor::new("DependentManager"),
        )];
        const DOWNSTREAM: GameDataManagerShape = GameDataManagerShape::new(
            "DownstreamManager",
            ManagerShapeKind::ComposedResource,
            DOWNSTREAM_INPUTS,
        );
        let catalog = build_manager_catalog(ManagerCatalogInput::new(
            &[DEPENDS_ON_ITEM_PROVIDER, FAMILY_READER, DOWNSTREAM],
            TABLES,
            FAMILIES,
            &[ITEM_SCHEMA, NPC_SCHEMA, MTX_ITEM_SCHEMA],
        ))
        .expect("catalog should resolve");

        let impact = catalog.delete_impact(ITEM_TARGET);

        assert_eq!(impact.target(), ITEM_TARGET);
        assert_eq!(
            impact.affected_targets(),
            &[ITEM_TARGET, ITEM_FAMILY_TARGET]
        );
        assert_eq!(
            impact.broken(),
            &[
                ManagerNodeId::Provider(ITEM_TARGET),
                ManagerNodeId::Explicit("DependentManager"),
                ManagerNodeId::Explicit("FamilyReader"),
            ]
        );
        assert_eq!(
            impact.invalidated(),
            &[ManagerNodeId::Explicit("DownstreamManager")]
        );
    }
}
