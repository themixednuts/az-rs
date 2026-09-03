//! Stable fingerprints for generated `GameData` manager projections.

use crate::release::{ProjectionHash, SchemaHash};

use super::manager_graph::{ManagerNode, ManagerNodeId};
use crate::manager::{
    DuplicateKeyPolicy, GameDataManagerInput, GameDataManagerShape, KeyKind, KeyPolicy,
    KeyTransform, ManagerShapeKind, ProductInputFormat, ProjectionTransformKind, ProviderTarget,
    RowFilterPredicate, SecondaryIndexKeyKind, SecondaryIndexStorage, TableFamilyDescriptor,
    TableInputDescriptor,
};

/// Schema input folded into a manager projection fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagerProjectionSource {
    target: ProviderTarget,
    schema_hash: SchemaHash,
}

impl ManagerProjectionSource {
    /// Creates a projection source hash for one provider target.
    #[inline]
    #[must_use]
    pub const fn new(target: ProviderTarget, schema_hash: SchemaHash) -> Self {
        Self {
            target,
            schema_hash,
        }
    }

    /// Provider target this schema hash belongs to.
    #[inline]
    #[must_use]
    pub const fn target(self) -> ProviderTarget {
        self.target
    }

    /// Stable table schema hash.
    #[inline]
    #[must_use]
    pub const fn schema_hash(self) -> SchemaHash {
        self.schema_hash
    }
}

/// Resolved manager/provider dependency folded into a manager projection
/// fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManagerProjectionDependency {
    id: ManagerNodeId,
    projection_hash: ProjectionHash,
}

impl ManagerProjectionDependency {
    /// Creates a dependency fingerprint input for a resolved upstream manager.
    #[inline]
    #[must_use]
    pub const fn new(id: ManagerNodeId, projection_hash: ProjectionHash) -> Self {
        Self {
            id,
            projection_hash,
        }
    }

    /// Resolved upstream manager identity.
    #[inline]
    #[must_use]
    pub const fn id(self) -> ManagerNodeId {
        self.id
    }

    /// Projection hash produced by the upstream manager.
    #[inline]
    #[must_use]
    pub const fn projection_hash(self) -> ProjectionHash {
        self.projection_hash
    }
}

/// Computes a stable projection fingerprint for a resolved manager node.
///
/// The fingerprint is explicit and stable across processes. It deliberately
/// does not use [`std::hash::Hash`], whose output is not a persistence contract.
#[must_use]
pub fn manager_projection_fingerprint(
    node: ManagerNode,
    sources: &[ManagerProjectionSource],
) -> ProjectionHash {
    manager_projection_fingerprint_inner(
        b"azoth.gamedata.manager.projection.v1\0",
        node,
        sources,
        &[],
    )
}

/// Computes a stable projection fingerprint including resolved upstream
/// manager/provider projection hashes.
///
/// Direct table/family inputs should be passed as [`ManagerProjectionSource`].
/// Manager/provider graph dependencies should be passed as
/// [`ManagerProjectionDependency`] so changes to an explicit provider's key
/// policy, filters, or projection transforms dirty downstream products.
#[must_use]
pub fn manager_projection_fingerprint_with_deps(
    node: ManagerNode,
    sources: &[ManagerProjectionSource],
    dependencies: &[ManagerProjectionDependency],
) -> ProjectionHash {
    manager_projection_fingerprint_inner(
        b"azoth.gamedata.manager.projection.v2\0",
        node,
        sources,
        dependencies,
    )
}

fn manager_projection_fingerprint_inner(
    domain: &[u8],
    node: ManagerNode,
    sources: &[ManagerProjectionSource],
    dependencies: &[ManagerProjectionDependency],
) -> ProjectionHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    match node {
        ManagerNode::Explicit(shape) => {
            hasher.update(&[1]);
            hash_manager_shape(&mut hasher, &shape);
        }
        ManagerNode::DefaultProvider(target) => {
            hasher.update(&[2]);
            hash_provider_target(&mut hasher, target);
            hash_key_policy(&mut hasher, KeyPolicy::row_handle());
            hash_duplicate_key_policy(&mut hasher, DuplicateKeyPolicy::Error);
        }
    }

    let mut sorted_sources = sources.to_vec();
    sorted_sources.sort_by(|left, right| {
        left.target
            .cmp(&right.target)
            .then_with(|| left.schema_hash.0.cmp(&right.schema_hash.0))
    });
    hasher.update(&(sorted_sources.len() as u64).to_le_bytes());
    for source in sorted_sources {
        hash_provider_target(&mut hasher, source.target);
        hasher.update(&source.schema_hash.0.to_le_bytes());
    }

    let mut sorted_dependencies = dependencies.to_vec();
    sorted_dependencies.sort_by_key(|dependency| dependency.id);
    hasher.update(&(sorted_dependencies.len() as u64).to_le_bytes());
    for dependency in sorted_dependencies {
        hash_manager_node_id(&mut hasher, dependency.id);
        hasher.update(&dependency.projection_hash.0);
    }

    ProjectionHash(*hasher.finalize().as_bytes())
}

fn hash_manager_shape(hasher: &mut blake3::Hasher, shape: &GameDataManagerShape) {
    hash_str(hasher, shape.name());
    hash_manager_shape_kind(hasher, shape.kind());
    hash_optional_provider_target(hasher, shape.provides());
    hasher.update(&(shape.inputs().len() as u64).to_le_bytes());
    for input in shape.inputs() {
        hash_manager_input(hasher, *input);
    }
    hash_key_policy(hasher, shape.key_policy());
    hash_duplicate_key_policy(hasher, shape.duplicate_key_policy());
    hasher.update(&(shape.row_filters().len() as u64).to_le_bytes());
    for filter in shape.row_filters() {
        hash_str(hasher, filter.field());
        hash_row_filter_predicate(hasher, filter.predicate());
        hash_optional_str(hasher, filter.compare_field());
    }
    hasher.update(&(shape.projection_transforms().len() as u64).to_le_bytes());
    for transform in shape.projection_transforms() {
        hash_str(hasher, transform.field());
        hash_optional_str(hasher, transform.source_column());
        hash_projection_transform_kind(hasher, transform.kind());
    }
    hasher.update(&(shape.secondary_indexes().len() as u64).to_le_bytes());
    for index in shape.secondary_indexes() {
        hash_str(hasher, index.name());
        hash_str(hasher, index.field());
        hash_secondary_index_key_kind(hasher, index.key_kind());
        hash_secondary_index_storage(hasher, index.storage());
        hash_duplicate_key_policy(hasher, index.duplicate_key_policy());
    }
}

fn hash_manager_input(hasher: &mut blake3::Hasher, input: GameDataManagerInput) {
    match input {
        GameDataManagerInput::Table(table) => {
            hasher.update(&[1]);
            hash_table(hasher, table);
        }
        GameDataManagerInput::TableFamily(family) => {
            hasher.update(&[2]);
            hash_family(hasher, family);
        }
        GameDataManagerInput::Product(product) => {
            hasher.update(&[3]);
            hash_product_format(hasher, product.format());
            hash_str(hasher, product.asset_path());
            hash_str(hasher, product.rust_type());
        }
        GameDataManagerInput::Manager(manager) => {
            hasher.update(&[4]);
            hash_str(hasher, manager.manager());
        }
        GameDataManagerInput::Provider(provider) => {
            hasher.update(&[5]);
            hash_provider_target(hasher, provider.target());
        }
        GameDataManagerInput::SourceSchema(schema) => {
            hasher.update(&[6]);
            hash_str(hasher, schema.schema_type());
            hash_source_schema_role(hasher, schema.role());
        }
    }
}

fn hash_table(hasher: &mut blake3::Hasher, table: TableInputDescriptor) {
    hash_str(hasher, table.table_name());
    hash_str(hasher, table.row_name());
}

fn hash_family(hasher: &mut blake3::Hasher, family: TableFamilyDescriptor) {
    hash_str(hasher, family.name());
    hash_str(hasher, family.row_name());
    hash_duplicate_key_policy(hasher, family.duplicate_key_policy());
    hasher.update(&(family.tables().len() as u64).to_le_bytes());
    for table in family.tables() {
        hash_table(hasher, *table);
    }
}

fn hash_optional_provider_target(hasher: &mut blake3::Hasher, target: Option<ProviderTarget>) {
    match target {
        Some(target) => {
            hasher.update(&[1]);
            hash_provider_target(hasher, target);
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn hash_manager_node_id(hasher: &mut blake3::Hasher, id: ManagerNodeId) {
    match id {
        ManagerNodeId::Explicit(name) => {
            hasher.update(&[1]);
            hash_str(hasher, name);
        }
        ManagerNodeId::Provider(target) => {
            hasher.update(&[2]);
            hash_provider_target(hasher, target);
        }
    }
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

fn hash_key_policy(hasher: &mut blake3::Hasher, policy: KeyPolicy) {
    hash_key_kind(hasher, policy.kind());
    hasher.update(&(policy.transforms().len() as u64).to_le_bytes());
    for transform in policy.transforms() {
        hash_key_transform(hasher, *transform);
    }
    hasher.update(&[u8::from(policy.rejects_zero_crc())]);
    hasher.update(&[u8::from(policy.stores_key_text())]);
}

fn hash_manager_shape_kind(hasher: &mut blake3::Hasher, kind: ManagerShapeKind) {
    match kind {
        ManagerShapeKind::SingleTableIndex => {
            hasher.update(&[1]);
        }
        ManagerShapeKind::TableFamilyIndex => {
            hasher.update(&[2]);
        }
        ManagerShapeKind::CrcKeyProjection => {
            hasher.update(&[3]);
        }
        ManagerShapeKind::EnumKeyProjection => {
            hasher.update(&[4]);
        }
        ManagerShapeKind::NumericKeyProjection => {
            hasher.update(&[5]);
        }
        ManagerShapeKind::StringKeyProjection => {
            hasher.update(&[6]);
        }
        ManagerShapeKind::PartitionedProjection => {
            hasher.update(&[7]);
        }
        ManagerShapeKind::FallbackProjection => {
            hasher.update(&[8]);
        }
        ManagerShapeKind::RowProjection => {
            hasher.update(&[9]);
        }
        ManagerShapeKind::ProductAssetResource => {
            hasher.update(&[10]);
        }
        ManagerShapeKind::ComposedResource => {
            hasher.update(&[11]);
        }
        ManagerShapeKind::Custom(value) => {
            hasher.update(&[12]);
            hash_str(hasher, value);
        }
    }
}

fn hash_duplicate_key_policy(hasher: &mut blake3::Hasher, policy: DuplicateKeyPolicy) {
    match policy {
        DuplicateKeyPolicy::Error => {
            hasher.update(&[1]);
        }
        DuplicateKeyPolicy::FirstWins => {
            hasher.update(&[2]);
        }
        DuplicateKeyPolicy::Overwrite => {
            hasher.update(&[3]);
        }
        DuplicateKeyPolicy::Multi => {
            hasher.update(&[4]);
        }
    }
}

fn hash_key_kind(hasher: &mut blake3::Hasher, kind: KeyKind) {
    match kind {
        KeyKind::RowHandle => {
            hasher.update(&[1]);
        }
        KeyKind::Crc32 => {
            hasher.update(&[2]);
        }
        KeyKind::Enum => {
            hasher.update(&[3]);
        }
        KeyKind::Numeric => {
            hasher.update(&[4]);
        }
        KeyKind::String => {
            hasher.update(&[5]);
        }
        KeyKind::Custom(value) => {
            hasher.update(&[6]);
            hash_str(hasher, value);
        }
    }
}

fn hash_key_transform(hasher: &mut blake3::Hasher, transform: KeyTransform) {
    match transform {
        KeyTransform::Trim => {
            hasher.update(&[1]);
        }
        KeyTransform::Lowercase => {
            hasher.update(&[2]);
        }
        KeyTransform::RemoveSpaceAndTab => {
            hasher.update(&[3]);
        }
        KeyTransform::Crc32 => {
            hasher.update(&[4]);
        }
        KeyTransform::Custom(value) => {
            hasher.update(&[5]);
            hash_str(hasher, value);
        }
    }
}

fn hash_row_filter_predicate(hasher: &mut blake3::Hasher, predicate: RowFilterPredicate) {
    match predicate {
        RowFilterPredicate::FieldFalse => {
            hasher.update(&[1]);
        }
        RowFilterPredicate::FieldTrue => {
            hasher.update(&[2]);
        }
        RowFilterPredicate::BoolTrueWhenPresent => {
            hasher.update(&[3]);
        }
        RowFilterPredicate::F32GreaterThanOrEqualZero => {
            hasher.update(&[4]);
        }
        RowFilterPredicate::F32LessThanOrEqualZero => {
            hasher.update(&[5]);
        }
        RowFilterPredicate::F32AnyGreaterThanZero => {
            hasher.update(&[6]);
        }
        RowFilterPredicate::I32LessThanOrEqualZero => {
            hasher.update(&[7]);
        }
        RowFilterPredicate::LowercaseCrcStringNonZero => {
            hasher.update(&[8]);
        }
        RowFilterPredicate::StringNotEqualToField => {
            hasher.update(&[9]);
        }
        RowFilterPredicate::Custom(value) => {
            hasher.update(&[10]);
            hash_str(hasher, value);
        }
    }
}

fn hash_projection_transform_kind(hasher: &mut blake3::Hasher, kind: ProjectionTransformKind) {
    match kind {
        ProjectionTransformKind::Identity => {
            hasher.update(&[1]);
        }
        ProjectionTransformKind::PlusJoinedList => {
            hasher.update(&[2]);
        }
        ProjectionTransformKind::NonEmptyString => {
            hasher.update(&[3]);
        }
        ProjectionTransformKind::OptionalFirstString => {
            hasher.update(&[4]);
        }
        ProjectionTransformKind::Crc32 => {
            hasher.update(&[5]);
        }
        ProjectionTransformKind::LowercaseCrcString => {
            hasher.update(&[6]);
        }
        ProjectionTransformKind::CrcList => {
            hasher.update(&[7]);
        }
        ProjectionTransformKind::TrimmedLowercaseCrcStringList => {
            hasher.update(&[8]);
        }
        ProjectionTransformKind::ForeignKeyTargetKey => {
            hasher.update(&[9]);
        }
        ProjectionTransformKind::ForeignKeyTargetLowercaseCrc => {
            hasher.update(&[10]);
        }
        ProjectionTransformKind::ForeignKeyRow => {
            hasher.update(&[11]);
        }
        ProjectionTransformKind::ForeignKeyRowList => {
            hasher.update(&[12]);
        }
        ProjectionTransformKind::F32RangeInclusive => {
            hasher.update(&[13]);
        }
        ProjectionTransformKind::F32MinutesToSeconds => {
            hasher.update(&[14]);
        }
        ProjectionTransformKind::U32ToU16BelowMax => {
            hasher.update(&[15]);
        }
        ProjectionTransformKind::Custom(value) => {
            hasher.update(&[16]);
            hash_str(hasher, value);
        }
    }
}

fn hash_secondary_index_key_kind(hasher: &mut blake3::Hasher, kind: SecondaryIndexKeyKind) {
    match kind {
        SecondaryIndexKeyKind::U16 => {
            hasher.update(&[1]);
        }
        SecondaryIndexKeyKind::NonZeroU32 => {
            hasher.update(&[2]);
        }
        SecondaryIndexKeyKind::Crc32 => {
            hasher.update(&[3]);
        }
        SecondaryIndexKeyKind::Custom(value) => {
            hasher.update(&[4]);
            hash_str(hasher, value);
        }
    }
}

fn hash_secondary_index_storage(hasher: &mut blake3::Hasher, storage: SecondaryIndexStorage) {
    match storage {
        SecondaryIndexStorage::HashMap => {
            hasher.update(&[1]);
        }
        SecondaryIndexStorage::SparseVec => {
            hasher.update(&[2]);
        }
        SecondaryIndexStorage::DescendingF32 => {
            hasher.update(&[3]);
        }
        SecondaryIndexStorage::Custom(value) => {
            hasher.update(&[4]);
            hash_str(hasher, value);
        }
    }
}

fn hash_product_format(hasher: &mut blake3::Hasher, format: ProductInputFormat) {
    match format {
        ProductInputFormat::ObjectStream => {
            hasher.update(&[1]);
        }
        ProductInputFormat::Xml => {
            hasher.update(&[2]);
        }
        ProductInputFormat::Binary => {
            hasher.update(&[3]);
        }
        ProductInputFormat::Other(value) => {
            hasher.update(&[4]);
            hash_str(hasher, value);
        }
    }
}

fn hash_source_schema_role(hasher: &mut blake3::Hasher, role: crate::manager::SourceSchemaRole) {
    match role {
        crate::manager::SourceSchemaRole::Prefab => {
            hasher.update(&[1]);
        }
        crate::manager::SourceSchemaRole::StaticData => {
            hasher.update(&[2]);
        }
        crate::manager::SourceSchemaRole::Config => {
            hasher.update(&[3]);
        }
        crate::manager::SourceSchemaRole::Other(value) => {
            hasher.update(&[4]);
            hash_str(hasher, value);
        }
    }
}

fn hash_optional_str(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hash_str(hasher, value);
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn hash_str(hasher: &mut blake3::Hasher, value: &str) {
    hasher.update(&(value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::{GameDataManagerInput, KeyKind, KeyPolicy, ManagerShapeKind};

    const ITEM_TABLE: TableInputDescriptor = TableInputDescriptor::new("ItemTable", "ItemRow");
    const ITEM_TARGET: ProviderTarget = ProviderTarget::table(ITEM_TABLE);
    const SOURCE: ManagerProjectionSource =
        ManagerProjectionSource::new(ITEM_TARGET, SchemaHash(1));
    const SOURCE_CHANGED: ManagerProjectionSource =
        ManagerProjectionSource::new(ITEM_TARGET, SchemaHash(2));
    const TABLE_INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Table(ITEM_TABLE)];
    const EXPLICIT: GameDataManagerShape = GameDataManagerShape::new(
        "ItemManager",
        ManagerShapeKind::SingleTableIndex,
        TABLE_INPUTS,
    )
    .with_provides(ITEM_TARGET);
    const EXPLICIT_CRC: GameDataManagerShape = GameDataManagerShape::new(
        "ItemManager",
        ManagerShapeKind::SingleTableIndex,
        TABLE_INPUTS,
    )
    .with_provides(ITEM_TARGET)
    .with_key_policy(KeyPolicy::new(KeyKind::Crc32));

    #[test]
    fn default_provider_fingerprint_is_stable_and_schema_sensitive() {
        let node = ManagerNode::DefaultProvider(ITEM_TARGET);
        let first = manager_projection_fingerprint(node, &[SOURCE]);
        let second = manager_projection_fingerprint(node, &[SOURCE]);
        let changed_schema = manager_projection_fingerprint(node, &[SOURCE_CHANGED]);

        assert_eq!(first, second);
        assert_ne!(first, changed_schema);
    }

    #[test]
    fn explicit_projection_fingerprint_is_policy_sensitive() {
        let row_handle = manager_projection_fingerprint(ManagerNode::Explicit(EXPLICIT), &[SOURCE]);
        let crc = manager_projection_fingerprint(ManagerNode::Explicit(EXPLICIT_CRC), &[SOURCE]);

        assert_ne!(row_handle, crc);
    }

    #[test]
    fn source_order_does_not_affect_projection_fingerprint() {
        const NPC_TABLE: TableInputDescriptor = TableInputDescriptor::new("NpcTable", "NpcRow");
        const NPC_TARGET: ProviderTarget = ProviderTarget::table(NPC_TABLE);
        const NPC_SOURCE: ManagerProjectionSource =
            ManagerProjectionSource::new(NPC_TARGET, SchemaHash(7));
        let node = ManagerNode::Explicit(EXPLICIT);

        assert_eq!(
            manager_projection_fingerprint(node, &[SOURCE, NPC_SOURCE]),
            manager_projection_fingerprint(node, &[NPC_SOURCE, SOURCE])
        );
    }
}
