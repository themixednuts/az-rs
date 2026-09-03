//! Descriptors for `GameData` manager projections.
//!
//! A manager shape describes how editor, build, and host tooling projects table
//! rows, table families, typed product assets, and other managers into a domain
//! view. It is deliberately separate from reflected row metadata: row types own
//! authored value shape, table envelopes own physical identity, and manager
//! shapes own projection policy.
//!
//! These are composition contracts, not authoring machinery, so they are
//! unconditional: a project's generated catalog names them to register itself,
//! and a runtime binary that links the catalog must be able to name them
//! without linking the RON compiler. The catalog, graph, and fingerprint
//! *algorithms* over these descriptors stay behind `authoring`, in
//! [`crate::authoring`].

use std::collections::BTreeSet;
use std::fmt;

const NO_KEY_TRANSFORMS: &[KeyTransform] = &[];
const LOWERCASE_TRANSFORMS: &[KeyTransform] = &[KeyTransform::Lowercase];

/// A concrete table input referenced by a manager or table family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableInputDescriptor {
    table_name: &'static str,
    row_name: &'static str,
}

impl TableInputDescriptor {
    /// Creates a manager input from physical table and merged row-schema names.
    #[inline]
    #[must_use]
    pub const fn new(table_name: &'static str, row_name: &'static str) -> Self {
        Self {
            table_name,
            row_name,
        }
    }

    /// The logical table name.
    #[inline]
    #[must_use]
    pub const fn table_name(self) -> &'static str {
        self.table_name
    }

    /// The logical row type name used by this table.
    #[inline]
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        self.row_name
    }
}

/// A set of concrete tables that share one authored row type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableFamilyDescriptor {
    name: &'static str,
    row_name: &'static str,
    tables: &'static [TableInputDescriptor],
    duplicate_key_policy: DuplicateKeyPolicy,
}

impl TableFamilyDescriptor {
    /// Creates a table-family descriptor with duplicate keys rejected.
    #[inline]
    #[must_use]
    pub const fn new(
        name: &'static str,
        row_name: &'static str,
        tables: &'static [TableInputDescriptor],
    ) -> Self {
        Self {
            name,
            row_name,
            tables,
            duplicate_key_policy: DuplicateKeyPolicy::Error,
        }
    }

    /// Sets the cross-table duplicate-key policy for this family.
    #[inline]
    #[must_use]
    pub const fn with_duplicate_key_policy(mut self, policy: DuplicateKeyPolicy) -> Self {
        self.duplicate_key_policy = policy;
        self
    }

    /// The family name used by editor/build descriptors.
    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// The row type shared by every table in the family.
    #[inline]
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        self.row_name
    }

    /// Concrete table members in deterministic order.
    #[inline]
    #[must_use]
    pub const fn tables(self) -> &'static [TableInputDescriptor] {
        self.tables
    }

    /// How duplicate keys across member tables resolve.
    #[inline]
    #[must_use]
    pub const fn duplicate_key_policy(self) -> DuplicateKeyPolicy {
        self.duplicate_key_policy
    }
}

/// Format family for a required non-table product input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProductInputFormat {
    /// Legacy or native ObjectStream-style typed product.
    ObjectStream,
    /// XML-backed typed product.
    Xml,
    /// Binary typed product.
    Binary,
    /// Any project-owned product format that has not been promoted to a common variant.
    Other(&'static str),
}

/// A required typed product input to a manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProductInputDescriptor {
    format: ProductInputFormat,
    asset_path: &'static str,
    rust_type: &'static str,
}

impl ProductInputDescriptor {
    /// Creates a required product input descriptor.
    #[inline]
    #[must_use]
    pub const fn new(
        format: ProductInputFormat,
        asset_path: &'static str,
        rust_type: &'static str,
    ) -> Self {
        Self {
            format,
            asset_path,
            rust_type,
        }
    }

    /// Product format family.
    #[inline]
    #[must_use]
    pub const fn format(self) -> ProductInputFormat {
        self.format
    }

    /// Catalog-relative asset path required by the manager.
    #[inline]
    #[must_use]
    pub const fn asset_path(self) -> &'static str {
        self.asset_path
    }

    /// Rust type that owns the loaded product value.
    #[inline]
    #[must_use]
    pub const fn rust_type(self) -> &'static str {
        self.rust_type
    }
}

/// A dependency on another manager descriptor/resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ManagerDependencyDescriptor {
    manager: &'static str,
}

impl ManagerDependencyDescriptor {
    /// Creates a dependency by manager type/name.
    #[inline]
    #[must_use]
    pub const fn new(manager: &'static str) -> Self {
        Self { manager }
    }

    /// The required manager type/name.
    #[inline]
    #[must_use]
    pub const fn manager(self) -> &'static str {
        self.manager
    }
}

/// Table or table-family output that can be provided by either an explicit
/// manager shape or a synthesized read-only manager projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderTarget {
    /// Provider for one concrete table source.
    Table {
        /// Logical table name.
        table_name: &'static str,
        /// Logical row type used by the table.
        row_name: &'static str,
    },
    /// Provider for a table family sharing one row type.
    Family {
        /// Logical table-family name.
        family_name: &'static str,
        /// Logical row type used by the family.
        row_name: &'static str,
    },
}

impl ProviderTarget {
    /// Creates a provider target for one concrete table.
    #[inline]
    #[must_use]
    pub const fn table(table: TableInputDescriptor) -> Self {
        Self::Table {
            table_name: table.table_name(),
            row_name: table.row_name(),
        }
    }

    /// Creates a provider target for a table family.
    #[inline]
    #[must_use]
    pub const fn family(family: TableFamilyDescriptor) -> Self {
        Self::Family {
            family_name: family.name(),
            row_name: family.row_name(),
        }
    }

    /// Primary target name used for validation and diagnostics.
    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Table { table_name, .. } => table_name,
            Self::Family { family_name, .. } => family_name,
        }
    }

    /// Row type name used by the target.
    #[inline]
    #[must_use]
    pub const fn row_name(self) -> &'static str {
        match self {
            Self::Table { row_name, .. } | Self::Family { row_name, .. } => row_name,
        }
    }

    /// Stable kind string used for diagnostics.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Table { .. } => "table provider",
            Self::Family { .. } => "family provider",
        }
    }
}

/// Dependency on the manager provider for a table or table family.
///
/// A provider dependency deliberately targets the data provision instead of a
/// concrete manager name, so callers remain stable when a table moves between a
/// synthesized read-only provider and an explicit manager implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProviderDependencyDescriptor {
    target: ProviderTarget,
}

impl ProviderDependencyDescriptor {
    /// Creates a provider dependency for a table or table-family target.
    #[inline]
    #[must_use]
    pub const fn new(target: ProviderTarget) -> Self {
        Self { target }
    }

    /// Target data provision.
    #[inline]
    #[must_use]
    pub const fn target(self) -> ProviderTarget {
        self.target
    }
}

/// Role for a related source schema input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SourceSchemaRole {
    /// Prefab or prefab-like authored source related to the manager.
    Prefab,
    /// Static authored data source related to the manager.
    StaticData,
    /// Authored manager configuration source.
    Config,
    /// Project-owned role not modeled by the common variants.
    Other(&'static str),
}

/// Related source-schema input used for editor workflow or build validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceSchemaInputDescriptor {
    schema_type: &'static str,
    role: SourceSchemaRole,
}

impl SourceSchemaInputDescriptor {
    /// Creates a related source-schema input descriptor.
    #[inline]
    #[must_use]
    pub const fn new(schema_type: &'static str, role: SourceSchemaRole) -> Self {
        Self { schema_type, role }
    }

    /// Source schema type string.
    #[inline]
    #[must_use]
    pub const fn schema_type(self) -> &'static str {
        self.schema_type
    }

    /// How the manager uses this source schema.
    #[inline]
    #[must_use]
    pub const fn role(self) -> SourceSchemaRole {
        self.role
    }
}

/// One declared input to a manager shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GameDataManagerInput {
    /// A concrete table source/product.
    Table(TableInputDescriptor),
    /// A set of concrete tables sharing one row type.
    TableFamily(TableFamilyDescriptor),
    /// A required typed non-table product asset.
    Product(ProductInputDescriptor),
    /// Another manager that must be present before this manager can build.
    Manager(ManagerDependencyDescriptor),
    /// The manager provider for a table or table family.
    Provider(ProviderDependencyDescriptor),
    /// A related source-schema workflow input.
    SourceSchema(SourceSchemaInputDescriptor),
}

impl GameDataManagerInput {
    const fn validation_key(self) -> InputValidationKey {
        match self {
            Self::Table(table) => InputValidationKey::new("table", table.table_name()),
            Self::TableFamily(family) => InputValidationKey::new("family", family.name()),
            Self::Product(product) => InputValidationKey::new("product", product.asset_path()),
            Self::Manager(manager) => InputValidationKey::new("manager", manager.manager()),
            Self::Provider(provider) => {
                InputValidationKey::new(provider.target().kind(), provider.target().name())
            }
            Self::SourceSchema(schema) => InputValidationKey::new("schema", schema.schema_type()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct InputValidationKey {
    kind: &'static str,
    value: &'static str,
}

impl InputValidationKey {
    const fn new(kind: &'static str, value: &'static str) -> Self {
        Self { kind, value }
    }
}

/// Broad manager shape archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ManagerShapeKind {
    /// Single concrete table index.
    SingleTableIndex,
    /// Table-family index over tables sharing a row type.
    TableFamilyIndex,
    /// Projection keyed by CRC.
    CrcKeyProjection,
    /// Projection keyed by enum value.
    EnumKeyProjection,
    /// Projection keyed by numeric value.
    NumericKeyProjection,
    /// Projection keyed by string.
    StringKeyProjection,
    /// Partitioned projection with table or group indexes.
    PartitionedProjection,
    /// Projection with primary and fallback keys.
    FallbackProjection,
    /// Row projection without a separate key index.
    RowProjection,
    /// Manager exposing typed non-table product assets.
    ProductAssetResource,
    /// Manager composed from tables, products, and other managers.
    ComposedResource,
    /// Project-owned manager kind not modeled by the common variants.
    Custom(&'static str),
}

/// Duplicate-key resolution policy for an index or table family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DuplicateKeyPolicy {
    /// Duplicate keys are invalid.
    Error,
    /// First indexed row wins.
    FirstWins,
    /// Later rows replace earlier rows.
    Overwrite,
    /// Duplicate keys produce a multi-value bucket.
    Multi,
}

/// Target representation for key lowering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyKind {
    /// Table-local row handle.
    RowHandle,
    /// CRC32 key.
    Crc32,
    /// Enum key.
    Enum,
    /// Numeric key.
    Numeric,
    /// String key.
    String,
    /// Project-owned key representation.
    Custom(&'static str),
}

/// One transform applied before key lookup or storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyTransform {
    /// Remove leading/trailing whitespace.
    Trim,
    /// Convert to lowercase before hashing or comparison.
    Lowercase,
    /// Remove spaces and tabs.
    RemoveSpaceAndTab,
    /// Hash the normalized value as CRC32.
    Crc32,
    /// Project-owned normalization transform.
    Custom(&'static str),
}

/// Key lowering policy for a manager index or projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPolicy {
    kind: KeyKind,
    transforms: &'static [KeyTransform],
    reject_zero_crc: bool,
    store_key_text: bool,
}

impl KeyPolicy {
    /// Creates a key policy with no transforms.
    #[inline]
    #[must_use]
    pub const fn new(kind: KeyKind) -> Self {
        Self {
            kind,
            transforms: NO_KEY_TRANSFORMS,
            reject_zero_crc: false,
            store_key_text: false,
        }
    }

    /// A table-local row-handle policy.
    #[inline]
    #[must_use]
    pub const fn row_handle() -> Self {
        Self::new(KeyKind::RowHandle)
    }

    /// A lowercase CRC32 policy.
    #[inline]
    #[must_use]
    pub const fn crc32_lowercase() -> Self {
        Self {
            kind: KeyKind::Crc32,
            transforms: LOWERCASE_TRANSFORMS,
            reject_zero_crc: false,
            store_key_text: false,
        }
    }

    /// Replaces the transform list.
    #[inline]
    #[must_use]
    pub const fn with_transforms(mut self, transforms: &'static [KeyTransform]) -> Self {
        self.transforms = transforms;
        self
    }

    /// Rejects a normalized CRC value of zero.
    #[inline]
    #[must_use]
    pub const fn reject_zero_crc(mut self, reject: bool) -> Self {
        self.reject_zero_crc = reject;
        self
    }

    /// Preserves source key text beside the lowered key.
    #[inline]
    #[must_use]
    pub const fn store_key_text(mut self, store: bool) -> Self {
        self.store_key_text = store;
        self
    }

    /// Target key representation.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> KeyKind {
        self.kind
    }

    /// Normalization/lowering transforms in order.
    #[inline]
    #[must_use]
    pub const fn transforms(self) -> &'static [KeyTransform] {
        self.transforms
    }

    /// Whether zero CRC is invalid.
    #[inline]
    #[must_use]
    pub const fn rejects_zero_crc(self) -> bool {
        self.reject_zero_crc
    }

    /// Whether lowered data preserves source key text.
    #[inline]
    #[must_use]
    pub const fn stores_key_text(self) -> bool {
        self.store_key_text
    }
}

/// Row-filter predicate used by a source transform or manager projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RowFilterPredicate {
    /// Include rows where the field is false or absent.
    FieldFalse,
    /// Include rows where the field is true.
    FieldTrue,
    /// Include rows where an optional bool is true when present.
    BoolTrueWhenPresent,
    /// Include rows where the field value is greater than or equal to zero.
    F32GreaterThanOrEqualZero,
    /// Include rows where the field value is less than or equal to zero.
    F32LessThanOrEqualZero,
    /// Include rows where any named value in a group is greater than zero.
    F32AnyGreaterThanZero,
    /// Include rows where the field value is less than or equal to zero.
    I32LessThanOrEqualZero,
    /// Include rows where a lowered CRC string is non-zero.
    LowercaseCrcStringNonZero,
    /// Include rows where a string field does not equal another field.
    StringNotEqualToField,
    /// Project-owned predicate.
    Custom(&'static str),
}

/// A declared row inclusion filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowFilter {
    field: &'static str,
    predicate: RowFilterPredicate,
    compare_field: Option<&'static str>,
}

impl RowFilter {
    /// Creates a row filter for one field.
    #[inline]
    #[must_use]
    pub const fn new(field: &'static str, predicate: RowFilterPredicate) -> Self {
        Self {
            field,
            predicate,
            compare_field: None,
        }
    }

    /// Creates a filter that excludes rows where a boolean field is true.
    #[inline]
    #[must_use]
    pub const fn field_false(field: &'static str) -> Self {
        Self::new(field, RowFilterPredicate::FieldFalse)
    }

    /// Adds the compared field for predicates that need one.
    #[inline]
    #[must_use]
    pub const fn with_compare_field(mut self, field: &'static str) -> Self {
        self.compare_field = Some(field);
        self
    }

    /// Field read by the predicate.
    #[inline]
    #[must_use]
    pub const fn field(self) -> &'static str {
        self.field
    }

    /// Predicate applied to the row.
    #[inline]
    #[must_use]
    pub const fn predicate(self) -> RowFilterPredicate {
        self.predicate
    }

    /// Optional second field used for comparisons.
    #[inline]
    #[must_use]
    pub const fn compare_field(self) -> Option<&'static str> {
        self.compare_field
    }
}

/// Transform kind applied from source columns to projected manager fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProjectionTransformKind {
    /// Copy the source value through unchanged.
    Identity,
    /// Parse or join a plus-delimited list.
    PlusJoinedList,
    /// Require a non-empty string.
    NonEmptyString,
    /// Use the first non-empty string.
    OptionalFirstString,
    /// Convert a value to CRC32.
    Crc32,
    /// Lowercase then convert to CRC32.
    LowercaseCrcString,
    /// Convert a list of values to CRC32 values.
    CrcList,
    /// Lowercase and trim a list of CRC string values.
    TrimmedLowercaseCrcStringList,
    /// Read the target key of a foreign key.
    ForeignKeyTargetKey,
    /// Read the lowercase CRC key of a foreign key target.
    ForeignKeyTargetLowercaseCrc,
    /// Resolve a foreign-key row.
    ForeignKeyRow,
    /// Resolve a list of foreign-key rows.
    ForeignKeyRowList,
    /// Convert a float range to inclusive bounds.
    F32RangeInclusive,
    /// Convert minutes to seconds.
    F32MinutesToSeconds,
    /// Convert `u32` to `u16` with an exclusive maximum.
    U32ToU16BelowMax,
    /// Project-owned transform.
    Custom(&'static str),
}

/// One projected field transform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProjectionTransform {
    field: &'static str,
    source_column: Option<&'static str>,
    kind: ProjectionTransformKind,
}

impl ProjectionTransform {
    /// Creates a projection transform for a field.
    #[inline]
    #[must_use]
    pub const fn new(
        field: &'static str,
        source_column: Option<&'static str>,
        kind: ProjectionTransformKind,
    ) -> Self {
        Self {
            field,
            source_column,
            kind,
        }
    }

    /// Projected field name.
    #[inline]
    #[must_use]
    pub const fn field(self) -> &'static str {
        self.field
    }

    /// Optional physical source column name.
    #[inline]
    #[must_use]
    pub const fn source_column(self) -> Option<&'static str> {
        self.source_column
    }

    /// Transform kind.
    #[inline]
    #[must_use]
    pub const fn kind(self) -> ProjectionTransformKind {
        self.kind
    }
}

/// Storage strategy for a secondary index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SecondaryIndexStorage {
    /// Hash map based index.
    HashMap,
    /// Sparse vector based index.
    SparseVec,
    /// Sorted descending `f32` threshold index.
    DescendingF32,
    /// Project-owned storage strategy.
    Custom(&'static str),
}

/// Key representation for a secondary index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SecondaryIndexKeyKind {
    /// `u16` key.
    U16,
    /// non-zero `u32` key.
    NonZeroU32,
    /// CRC32 key.
    Crc32,
    /// Project-owned key kind.
    Custom(&'static str),
}

/// Secondary lookup index produced by a manager projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SecondaryIndexDescriptor {
    name: &'static str,
    field: &'static str,
    key_kind: SecondaryIndexKeyKind,
    storage: SecondaryIndexStorage,
    duplicate_key_policy: DuplicateKeyPolicy,
}

impl SecondaryIndexDescriptor {
    /// Creates a secondary index descriptor.
    #[inline]
    #[must_use]
    pub const fn new(
        name: &'static str,
        field: &'static str,
        key_kind: SecondaryIndexKeyKind,
        storage: SecondaryIndexStorage,
    ) -> Self {
        Self {
            name,
            field,
            key_kind,
            storage,
            duplicate_key_policy: DuplicateKeyPolicy::Error,
        }
    }

    /// Sets duplicate-key behavior for this index.
    #[inline]
    #[must_use]
    pub const fn with_duplicate_key_policy(mut self, policy: DuplicateKeyPolicy) -> Self {
        self.duplicate_key_policy = policy;
        self
    }

    /// Index name.
    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Projected field indexed by this index.
    #[inline]
    #[must_use]
    pub const fn field(self) -> &'static str {
        self.field
    }

    /// Index key kind.
    #[inline]
    #[must_use]
    pub const fn key_kind(self) -> SecondaryIndexKeyKind {
        self.key_kind
    }

    /// Index storage strategy.
    #[inline]
    #[must_use]
    pub const fn storage(self) -> SecondaryIndexStorage {
        self.storage
    }

    /// Duplicate-key policy for this index.
    #[inline]
    #[must_use]
    pub const fn duplicate_key_policy(self) -> DuplicateKeyPolicy {
        self.duplicate_key_policy
    }
}

/// Static manager projection descriptor consumed by editor/build tooling.
///
/// This is also the registry entry a gem contributes: composing hosts own a
/// `Registry<GameDataManagerShape>` keyed on the manager name, so two gems
/// claiming one manager name is a composition error before the manager graph
/// is ever built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameDataManagerShape {
    name: &'static str,
    kind: ManagerShapeKind,
    inputs: &'static [GameDataManagerInput],
    provides: Option<ProviderTarget>,
    key_policy: KeyPolicy,
    duplicate_key_policy: DuplicateKeyPolicy,
    row_filters: &'static [RowFilter],
    projection_transforms: &'static [ProjectionTransform],
    secondary_indexes: &'static [SecondaryIndexDescriptor],
}

impl az_gem_contract::RegistryEntry for GameDataManagerShape {
    type Key = &'static str;
    type Requires = az_gem_contract::Unconditional;

    fn registry_name() -> &'static str {
        "gamedata-manager"
    }

    fn key(&self) -> &'static str {
        self.name
    }
}

impl GameDataManagerShape {
    /// Creates a manager descriptor with default policies and no transforms.
    #[inline]
    #[must_use]
    pub const fn new(
        name: &'static str,
        kind: ManagerShapeKind,
        inputs: &'static [GameDataManagerInput],
    ) -> Self {
        Self {
            name,
            kind,
            inputs,
            provides: None,
            key_policy: KeyPolicy::row_handle(),
            duplicate_key_policy: DuplicateKeyPolicy::Error,
            row_filters: &[],
            projection_transforms: &[],
            secondary_indexes: &[],
        }
    }

    /// Declares that this explicit manager is the provider for a table or table
    /// family. Provider dependencies resolve to this manager instead of a
    /// synthesized read-only projection.
    #[inline]
    #[must_use]
    pub const fn with_provides(mut self, target: ProviderTarget) -> Self {
        self.provides = Some(target);
        self
    }

    /// Sets the manager's primary key lowering policy.
    #[inline]
    #[must_use]
    pub const fn with_key_policy(mut self, policy: KeyPolicy) -> Self {
        self.key_policy = policy;
        self
    }

    /// Sets the primary duplicate-key policy.
    #[inline]
    #[must_use]
    pub const fn with_duplicate_key_policy(mut self, policy: DuplicateKeyPolicy) -> Self {
        self.duplicate_key_policy = policy;
        self
    }

    /// Sets row inclusion filters.
    #[inline]
    #[must_use]
    pub const fn with_row_filters(mut self, filters: &'static [RowFilter]) -> Self {
        self.row_filters = filters;
        self
    }

    /// Sets projected field transforms.
    #[inline]
    #[must_use]
    pub const fn with_projection_transforms(
        mut self,
        transforms: &'static [ProjectionTransform],
    ) -> Self {
        self.projection_transforms = transforms;
        self
    }

    /// Sets secondary indexes.
    #[inline]
    #[must_use]
    pub const fn with_secondary_indexes(
        mut self,
        indexes: &'static [SecondaryIndexDescriptor],
    ) -> Self {
        self.secondary_indexes = indexes;
        self
    }

    /// Validates descriptor invariants before editor/build publication.
    ///
    /// # Errors
    ///
    /// Returns an error when required names are empty, duplicate manager inputs
    /// or secondary indexes are declared, table-family rows do not match, or a
    /// predicate/transform misses a required field.
    pub fn validate(&self) -> Result<(), GameDataManagerShapeError> {
        if self.name.is_empty() {
            return Err(GameDataManagerShapeError::EmptyManagerName);
        }
        if self.inputs.is_empty() {
            return Err(GameDataManagerShapeError::MissingInputs { manager: self.name });
        }
        self.validate_inputs()?;
        self.validate_provides()?;
        self.validate_row_filters()?;
        self.validate_projection_transforms()?;
        self.validate_secondary_indexes()?;
        Ok(())
    }

    const fn validate_provides(&self) -> Result<(), GameDataManagerShapeError> {
        let Some(target) = self.provides else {
            return Ok(());
        };
        if target.name().is_empty() {
            return Err(GameDataManagerShapeError::EmptyInputName {
                manager: self.name,
                input_kind: target.kind(),
            });
        }
        if target.row_name().is_empty() {
            return Err(GameDataManagerShapeError::EmptyProviderRow {
                manager: self.name,
                target: target.name(),
            });
        }
        Ok(())
    }

    /// Manager type/name.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// Broad manager shape archetype.
    #[inline]
    #[must_use]
    pub const fn kind(&self) -> ManagerShapeKind {
        self.kind
    }

    /// Declared manager inputs.
    #[inline]
    #[must_use]
    pub const fn inputs(&self) -> &'static [GameDataManagerInput] {
        self.inputs
    }

    /// Data provision supplied by this explicit manager, if any.
    #[inline]
    #[must_use]
    pub const fn provides(&self) -> Option<ProviderTarget> {
        self.provides
    }

    /// Primary key lowering policy.
    #[inline]
    #[must_use]
    pub const fn key_policy(&self) -> KeyPolicy {
        self.key_policy
    }

    /// Primary duplicate-key behavior.
    #[inline]
    #[must_use]
    pub const fn duplicate_key_policy(&self) -> DuplicateKeyPolicy {
        self.duplicate_key_policy
    }

    /// Row filters applied by this projection.
    #[inline]
    #[must_use]
    pub const fn row_filters(&self) -> &'static [RowFilter] {
        self.row_filters
    }

    /// Projected field transforms.
    #[inline]
    #[must_use]
    pub const fn projection_transforms(&self) -> &'static [ProjectionTransform] {
        self.projection_transforms
    }

    /// Secondary indexes built by this manager.
    #[inline]
    #[must_use]
    pub const fn secondary_indexes(&self) -> &'static [SecondaryIndexDescriptor] {
        self.secondary_indexes
    }

    fn validate_inputs(&self) -> Result<(), GameDataManagerShapeError> {
        let mut seen = BTreeSet::new();
        for input in self.inputs {
            let key = input.validation_key();
            if key.value.is_empty() {
                return Err(GameDataManagerShapeError::EmptyInputName {
                    manager: self.name,
                    input_kind: key.kind,
                });
            }
            if !seen.insert(key) {
                return Err(GameDataManagerShapeError::DuplicateInput {
                    manager: self.name,
                    input_kind: key.kind,
                    input: key.value,
                });
            }
            match input {
                GameDataManagerInput::Table(table) => self.validate_table(*table)?,
                GameDataManagerInput::TableFamily(family) => self.validate_family(*family)?,
                GameDataManagerInput::Product(product) => self.validate_product(*product)?,
                GameDataManagerInput::Manager(manager) => {
                    if manager.manager() == self.name {
                        return Err(GameDataManagerShapeError::SelfDependency {
                            manager: self.name,
                        });
                    }
                }
                GameDataManagerInput::Provider(provider) => {
                    if provider.target().row_name().is_empty() {
                        return Err(GameDataManagerShapeError::EmptyProviderRow {
                            manager: self.name,
                            target: provider.target().name(),
                        });
                    }
                }
                GameDataManagerInput::SourceSchema(schema) => {
                    if matches!(schema.role(), SourceSchemaRole::Other(value) if value.is_empty()) {
                        return Err(GameDataManagerShapeError::EmptyInputName {
                            manager: self.name,
                            input_kind: "source schema role",
                        });
                    }
                }
            }
        }
        Ok(())
    }

    const fn validate_table(
        &self,
        table: TableInputDescriptor,
    ) -> Result<(), GameDataManagerShapeError> {
        if table.row_name().is_empty() {
            return Err(GameDataManagerShapeError::EmptyTableRow {
                manager: self.name,
                table: table.table_name(),
            });
        }
        Ok(())
    }

    fn validate_family(
        &self,
        family: TableFamilyDescriptor,
    ) -> Result<(), GameDataManagerShapeError> {
        if family.row_name().is_empty() {
            return Err(GameDataManagerShapeError::EmptyTableFamilyRow {
                manager: self.name,
                family: family.name(),
            });
        }
        if family.tables().is_empty() {
            return Err(GameDataManagerShapeError::MissingFamilyTables {
                manager: self.name,
                family: family.name(),
            });
        }
        for table in family.tables() {
            self.validate_table(*table)?;
            if table.row_name() != family.row_name() {
                return Err(GameDataManagerShapeError::TableFamilyRowMismatch {
                    manager: self.name,
                    family: family.name(),
                    table: table.table_name(),
                    expected: family.row_name(),
                    actual: table.row_name(),
                });
            }
        }
        Ok(())
    }

    const fn validate_product(
        &self,
        product: ProductInputDescriptor,
    ) -> Result<(), GameDataManagerShapeError> {
        if product.rust_type().is_empty() {
            return Err(GameDataManagerShapeError::EmptyProductType {
                manager: self.name,
                asset_path: product.asset_path(),
            });
        }
        if matches!(product.format(), ProductInputFormat::Other(value) if value.is_empty()) {
            return Err(GameDataManagerShapeError::EmptyInputName {
                manager: self.name,
                input_kind: "product format",
            });
        }
        Ok(())
    }

    fn validate_row_filters(&self) -> Result<(), GameDataManagerShapeError> {
        for filter in self.row_filters {
            if filter.field().is_empty() {
                return Err(GameDataManagerShapeError::EmptyFilterField { manager: self.name });
            }
            if matches!(
                filter.predicate(),
                RowFilterPredicate::StringNotEqualToField
            ) && filter.compare_field().is_none_or(str::is_empty)
            {
                return Err(GameDataManagerShapeError::MissingFilterCompareField {
                    manager: self.name,
                    field: filter.field(),
                });
            }
            if matches!(filter.predicate(), RowFilterPredicate::Custom(value) if value.is_empty()) {
                return Err(GameDataManagerShapeError::EmptyFilterPredicate {
                    manager: self.name,
                    field: filter.field(),
                });
            }
        }
        Ok(())
    }

    fn validate_projection_transforms(&self) -> Result<(), GameDataManagerShapeError> {
        let mut seen = BTreeSet::new();
        for transform in self.projection_transforms {
            if transform.field().is_empty() {
                return Err(GameDataManagerShapeError::EmptyProjectionField { manager: self.name });
            }
            if !seen.insert(transform.field()) {
                return Err(GameDataManagerShapeError::DuplicateProjectionField {
                    manager: self.name,
                    field: transform.field(),
                });
            }
            if transform.source_column().is_some_and(str::is_empty) {
                return Err(GameDataManagerShapeError::EmptyProjectionSource {
                    manager: self.name,
                    field: transform.field(),
                });
            }
            if matches!(transform.kind(), ProjectionTransformKind::Custom(value) if value.is_empty())
            {
                return Err(GameDataManagerShapeError::EmptyProjectionTransform {
                    manager: self.name,
                    field: transform.field(),
                });
            }
        }
        Ok(())
    }

    fn validate_secondary_indexes(&self) -> Result<(), GameDataManagerShapeError> {
        let mut seen = BTreeSet::new();
        for index in self.secondary_indexes {
            if index.name().is_empty() {
                return Err(GameDataManagerShapeError::EmptySecondaryIndexName {
                    manager: self.name,
                });
            }
            if index.field().is_empty() {
                return Err(GameDataManagerShapeError::EmptySecondaryIndexField {
                    manager: self.name,
                    index: index.name(),
                });
            }
            if !seen.insert(index.name()) {
                return Err(GameDataManagerShapeError::DuplicateSecondaryIndex {
                    manager: self.name,
                    index: index.name(),
                });
            }
            if matches!(index.key_kind(), SecondaryIndexKeyKind::Custom(value) if value.is_empty())
                || matches!(index.storage(), SecondaryIndexStorage::Custom(value) if value.is_empty())
            {
                return Err(
                    GameDataManagerShapeError::InvalidSecondaryIndexCustomValue {
                        manager: self.name,
                        index: index.name(),
                    },
                );
            }
        }
        Ok(())
    }
}

/// Descriptor validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GameDataManagerShapeError {
    /// Manager name is empty.
    #[error("GameData manager name is empty")]
    EmptyManagerName,
    /// Manager has no declared inputs.
    #[error("GameData manager `{manager}` must declare at least one input")]
    MissingInputs { manager: &'static str },
    /// An input's primary name is empty.
    #[error("GameData manager `{manager}` has an empty {input_kind} input name")]
    EmptyInputName {
        manager: &'static str,
        input_kind: &'static str,
    },
    /// The same input is declared twice.
    #[error("GameData manager `{manager}` declares duplicate {input_kind} input `{input}`")]
    DuplicateInput {
        manager: &'static str,
        input_kind: &'static str,
        input: &'static str,
    },
    /// A table input has no row type.
    #[error("GameData manager `{manager}` table `{table}` has an empty row type")]
    EmptyTableRow {
        manager: &'static str,
        table: &'static str,
    },
    /// A provider dependency has no row type.
    #[error("GameData manager `{manager}` provider target `{target}` has an empty row type")]
    EmptyProviderRow {
        manager: &'static str,
        target: &'static str,
    },
    /// A table family has no row type.
    #[error("GameData manager `{manager}` table family `{family}` has an empty row type")]
    EmptyTableFamilyRow {
        manager: &'static str,
        family: &'static str,
    },
    /// A table family has no concrete tables.
    #[error("GameData manager `{manager}` table family `{family}` must declare tables")]
    MissingFamilyTables {
        manager: &'static str,
        family: &'static str,
    },
    /// A family table uses a different row type than the family.
    #[error(
        "GameData manager `{manager}` table family `{family}` table `{table}` row `{actual}` does not match expected row `{expected}`"
    )]
    TableFamilyRowMismatch {
        manager: &'static str,
        family: &'static str,
        table: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
    /// A product input has no Rust type.
    #[error("GameData manager `{manager}` product `{asset_path}` has an empty Rust type")]
    EmptyProductType {
        manager: &'static str,
        asset_path: &'static str,
    },
    /// A manager depends on itself.
    #[error("GameData manager `{manager}` cannot depend on itself")]
    SelfDependency { manager: &'static str },
    /// A row filter does not name a field.
    #[error("GameData manager `{manager}` has a row filter with an empty field")]
    EmptyFilterField { manager: &'static str },
    /// A comparison filter is missing its comparison field.
    #[error("GameData manager `{manager}` row filter `{field}` is missing a comparison field")]
    MissingFilterCompareField {
        manager: &'static str,
        field: &'static str,
    },
    /// A custom predicate is empty.
    #[error("GameData manager `{manager}` row filter `{field}` has an empty custom predicate")]
    EmptyFilterPredicate {
        manager: &'static str,
        field: &'static str,
    },
    /// A projection transform does not name a field.
    #[error("GameData manager `{manager}` has a projection with an empty field")]
    EmptyProjectionField { manager: &'static str },
    /// Two projection transforms write the same field.
    #[error("GameData manager `{manager}` declares duplicate projected field `{field}`")]
    DuplicateProjectionField {
        manager: &'static str,
        field: &'static str,
    },
    /// A projection transform has an empty source column.
    #[error("GameData manager `{manager}` projected field `{field}` has an empty source column")]
    EmptyProjectionSource {
        manager: &'static str,
        field: &'static str,
    },
    /// A custom projection transform is empty.
    #[error("GameData manager `{manager}` projected field `{field}` has an empty custom transform")]
    EmptyProjectionTransform {
        manager: &'static str,
        field: &'static str,
    },
    /// A secondary index does not name itself.
    #[error("GameData manager `{manager}` has a secondary index with an empty name")]
    EmptySecondaryIndexName { manager: &'static str },
    /// A secondary index does not name a field.
    #[error("GameData manager `{manager}` secondary index `{index}` has an empty field")]
    EmptySecondaryIndexField {
        manager: &'static str,
        index: &'static str,
    },
    /// Two secondary indexes share a name.
    #[error("GameData manager `{manager}` declares duplicate secondary index `{index}`")]
    DuplicateSecondaryIndex {
        manager: &'static str,
        index: &'static str,
    },
    /// A secondary index has an empty project-owned key or storage value.
    #[error("GameData manager `{manager}` secondary index `{index}` has an empty custom value")]
    InvalidSecondaryIndexCustomValue {
        manager: &'static str,
        index: &'static str,
    },
}

impl fmt::Display for ProductInputFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ObjectStream => formatter.write_str("objectstream"),
            Self::Xml => formatter.write_str("xml"),
            Self::Binary => formatter.write_str("binary"),
            Self::Other(value) => formatter.write_str(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER_ITEMS: TableInputDescriptor = TableInputDescriptor::new("MasterItems", "ItemRow");
    const MTX_ITEMS: TableInputDescriptor = TableInputDescriptor::new("MasterItemsMtx", "ItemRow");
    const FAMILY_TABLES: &[TableInputDescriptor] = &[MASTER_ITEMS, MTX_ITEMS];
    const ITEM_FAMILY: TableFamilyDescriptor =
        TableFamilyDescriptor::new("ItemDefinition", "ItemRow", FAMILY_TABLES)
            .with_duplicate_key_policy(DuplicateKeyPolicy::Overwrite);
    const INPUTS: &[GameDataManagerInput] = &[
        GameDataManagerInput::TableFamily(ITEM_FAMILY),
        GameDataManagerInput::Product(ProductInputDescriptor::new(
            ProductInputFormat::ObjectStream,
            "sharedassets/genericassets/items/equiptypesdatabase.equipdb",
            "example_project::assets::equipment::EquipmentDatabaseAsset",
        )),
        GameDataManagerInput::Manager(ManagerDependencyDescriptor::new("DyeColorDataManager")),
    ];
    const KEY_TRANSFORMS: &[KeyTransform] = &[KeyTransform::Trim, KeyTransform::Lowercase];
    const ROW_FILTERS: &[RowFilter] = &[RowFilter::field_false("exclude_from_game")];
    const PROJECTIONS: &[ProjectionTransform] = &[
        ProjectionTransform::new(
            "item_id_crc",
            Some("ItemID"),
            ProjectionTransformKind::LowercaseCrcString,
        ),
        ProjectionTransform::new(
            "perk_rows",
            Some("ItemPerks"),
            ProjectionTransformKind::ForeignKeyRowList,
        ),
    ];
    const INDEXES: &[SecondaryIndexDescriptor] = &[SecondaryIndexDescriptor::new(
        "by_item_class",
        "item_class",
        SecondaryIndexKeyKind::Crc32,
        SecondaryIndexStorage::HashMap,
    )
    .with_duplicate_key_policy(DuplicateKeyPolicy::Multi)];
    const ITEM_MANAGER: GameDataManagerShape = GameDataManagerShape::new(
        "ItemDataManager",
        ManagerShapeKind::CrcKeyProjection,
        INPUTS,
    )
    .with_key_policy(
        KeyPolicy::new(KeyKind::Crc32)
            .with_transforms(KEY_TRANSFORMS)
            .reject_zero_crc(true)
            .store_key_text(true),
    )
    .with_duplicate_key_policy(DuplicateKeyPolicy::Overwrite)
    .with_row_filters(ROW_FILTERS)
    .with_projection_transforms(PROJECTIONS)
    .with_secondary_indexes(INDEXES);

    #[test]
    fn manager_shape_supports_static_generated_descriptors() {
        ITEM_MANAGER.validate().expect("valid manager descriptor");

        assert_eq!(ITEM_MANAGER.name(), "ItemDataManager");
        assert_eq!(ITEM_MANAGER.inputs().len(), 3);
        assert_eq!(ITEM_MANAGER.key_policy().kind(), KeyKind::Crc32);
        assert_eq!(
            ITEM_MANAGER.key_policy().transforms(),
            &[KeyTransform::Trim, KeyTransform::Lowercase]
        );
        assert!(ITEM_MANAGER.key_policy().rejects_zero_crc());
        assert!(ITEM_MANAGER.key_policy().stores_key_text());
        assert_eq!(
            ITEM_MANAGER.duplicate_key_policy(),
            DuplicateKeyPolicy::Overwrite
        );
        assert_eq!(ITEM_MANAGER.projection_transforms().len(), 2);
        assert_eq!(
            ITEM_MANAGER.secondary_indexes()[0].duplicate_key_policy(),
            DuplicateKeyPolicy::Multi
        );
    }

    #[test]
    fn validation_rejects_family_row_mismatch() {
        const WRONG_TABLE: TableInputDescriptor = TableInputDescriptor::new("Wrong", "WrongRow");
        const TABLES: &[TableInputDescriptor] = &[MASTER_ITEMS, WRONG_TABLE];
        const BAD_FAMILY: TableFamilyDescriptor =
            TableFamilyDescriptor::new("ItemDefinition", "ItemRow", TABLES);
        const BAD_INPUTS: &[GameDataManagerInput] =
            &[GameDataManagerInput::TableFamily(BAD_FAMILY)];
        const SHAPE: GameDataManagerShape = GameDataManagerShape::new(
            "BadItemDataManager",
            ManagerShapeKind::TableFamilyIndex,
            BAD_INPUTS,
        );

        assert!(matches!(
            SHAPE.validate(),
            Err(GameDataManagerShapeError::TableFamilyRowMismatch {
                family: "ItemDefinition",
                table: "Wrong",
                expected: "ItemRow",
                actual: "WrongRow",
                ..
            })
        ));
    }

    #[test]
    fn validation_rejects_self_dependency() {
        const INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Manager(
            ManagerDependencyDescriptor::new("LoopManager"),
        )];
        const SHAPE: GameDataManagerShape =
            GameDataManagerShape::new("LoopManager", ManagerShapeKind::ComposedResource, INPUTS);

        assert!(matches!(
            SHAPE.validate(),
            Err(GameDataManagerShapeError::SelfDependency {
                manager: "LoopManager"
            })
        ));
    }

    #[test]
    fn validation_rejects_incomplete_comparison_filter() {
        const INPUTS: &[GameDataManagerInput] = &[GameDataManagerInput::Table(MASTER_ITEMS)];
        const FILTERS: &[RowFilter] = &[RowFilter::new(
            "primary_id",
            RowFilterPredicate::StringNotEqualToField,
        )];
        const SHAPE: GameDataManagerShape =
            GameDataManagerShape::new("CompareManager", ManagerShapeKind::SingleTableIndex, INPUTS)
                .with_row_filters(FILTERS);

        assert!(matches!(
            SHAPE.validate(),
            Err(GameDataManagerShapeError::MissingFilterCompareField {
                manager: "CompareManager",
                field: "primary_id"
            })
        ));
    }
}
