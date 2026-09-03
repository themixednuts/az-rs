//! Descriptor-backed `GameData` row schemas.
//!
//! A row schema describes one merged logical row type. Physical table identity
//! belongs to the authored RON envelope and compiled asset header, never to the
//! generated Rust schema.

use std::collections::{BTreeMap, BTreeSet};

use az_core::crc::Crc32;
use thiserror::Error;

use crate::release::SchemaHash;
use crate::table::{AtomType, CellType, ListElementType, PairType, RangeType};
use crate::table_set::{ColumnSchema, EnumVariantMeta, ForeignKeyMeta};

#[cfg(feature = "authoring")]
pub(crate) trait AuthoringTableSchema: std::fmt::Debug + Sync {
    fn name(&self) -> &str;
    fn crc(&self) -> u32;
    fn row_crc(&self) -> u32;
    fn column_count(&self) -> usize;
    fn column(&self, index: usize) -> Option<ColumnSchema>;
    fn columns(&self) -> AuthoringColumns<'_>;
    fn schema_hash(&self) -> SchemaHash;
}

#[cfg(feature = "authoring")]
pub(crate) struct AuthoringColumns<'a> {
    table: &'a dyn AuthoringTableSchema,
    next_index: usize,
}

#[cfg(feature = "authoring")]
impl Iterator for AuthoringColumns<'_> {
    type Item = ColumnSchema;

    fn next(&mut self) -> Option<Self::Item> {
        let column = self.table.column(self.next_index)?;
        self.next_index += 1;
        Some(column)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.table.column_count().saturating_sub(self.next_index);
        (remaining, Some(remaining))
    }
}

#[cfg(feature = "authoring")]
impl ExactSizeIterator for AuthoringColumns<'_> {}
#[cfg(feature = "authoring")]
impl std::iter::FusedIterator for AuthoringColumns<'_> {}

/// Enum variant metadata used by authored-source validation.
pub type EnumVariantDescriptor = EnumVariantMeta;

/// Foreign-key target metadata used by authored-source validation.
pub type ForeignKeyTargetDescriptor = ForeignKeyMeta;

/// One typed column in a merged `GameData` row schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnSchemaDescriptor {
    field_name: &'static str,
    source_column_name: &'static str,
    cell_type: CellType,
    key_candidate: bool,
    required: bool,
    enum_variants: &'static [EnumVariantDescriptor],
    foreign_key_targets: &'static [ForeignKeyTargetDescriptor],
}

impl ColumnSchemaDescriptor {
    #[inline]
    #[must_use]
    pub const fn new(
        field_name: &'static str,
        source_column_name: &'static str,
        cell_type: CellType,
    ) -> Self {
        Self {
            field_name,
            source_column_name,
            cell_type,
            key_candidate: false,
            required: true,
            enum_variants: &[],
            foreign_key_targets: &[],
        }
    }

    #[inline]
    #[must_use]
    pub const fn key_candidate(mut self, key_candidate: bool) -> Self {
        self.key_candidate = key_candidate;
        self
    }

    #[inline]
    #[must_use]
    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    #[inline]
    #[must_use]
    pub const fn optional(self) -> Self {
        self.required(false)
    }

    #[inline]
    #[must_use]
    pub const fn with_enum_variants(
        mut self,
        enum_variants: &'static [EnumVariantDescriptor],
    ) -> Self {
        self.enum_variants = enum_variants;
        self
    }

    #[inline]
    #[must_use]
    pub const fn with_foreign_key_targets(
        mut self,
        foreign_key_targets: &'static [ForeignKeyTargetDescriptor],
    ) -> Self {
        self.foreign_key_targets = foreign_key_targets;
        self
    }

    #[inline]
    #[must_use]
    pub const fn field_name(self) -> &'static str {
        self.field_name
    }

    #[inline]
    #[must_use]
    pub const fn source_column_name(self) -> &'static str {
        self.source_column_name
    }

    #[inline]
    #[must_use]
    pub const fn cell_type(self) -> CellType {
        self.cell_type
    }

    #[inline]
    #[must_use]
    pub const fn is_key_candidate(self) -> bool {
        self.key_candidate
    }

    #[inline]
    #[must_use]
    pub const fn is_required(self) -> bool {
        self.required
    }

    #[inline]
    #[must_use]
    pub const fn enum_variants(self) -> &'static [EnumVariantDescriptor] {
        self.enum_variants
    }

    #[inline]
    #[must_use]
    pub const fn foreign_key_targets(self) -> &'static [ForeignKeyTargetDescriptor] {
        self.foreign_key_targets
    }

    #[inline]
    #[must_use]
    pub const fn source_column_crc(self) -> u32 {
        Crc32::from_str_lower(self.source_column_name).value()
    }
}

/// One merged logical `GameData` row schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSchemaDescriptor {
    name: &'static str,
    columns: &'static [ColumnSchemaDescriptor],
}

impl RowSchemaDescriptor {
    #[inline]
    #[must_use]
    pub const fn new(name: &'static str, columns: &'static [ColumnSchemaDescriptor]) -> Self {
        Self { name, columns }
    }

    #[inline]
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    #[inline]
    #[must_use]
    pub const fn name_crc(self) -> u32 {
        Crc32::from_str_lower(self.name).value()
    }

    #[inline]
    #[must_use]
    pub const fn columns(self) -> &'static [ColumnSchemaDescriptor] {
        self.columns
    }

    #[inline]
    #[must_use]
    pub const fn column_count(self) -> usize {
        self.columns.len()
    }

    #[inline]
    #[must_use]
    pub fn column(self, index: usize) -> Option<ColumnSchemaDescriptor> {
        self.columns.get(index).copied()
    }

    pub fn key_candidates(self) -> impl Iterator<Item = ColumnSchemaDescriptor> {
        self.columns
            .iter()
            .copied()
            .filter(|column| column.is_key_candidate())
    }

    #[must_use]
    pub fn column_by_field_name(self, field_name: &str) -> Option<ColumnSchemaDescriptor> {
        self.columns
            .iter()
            .copied()
            .find(|column| column.field_name() == field_name)
    }

    #[must_use]
    pub fn column_by_source_column_name(
        self,
        source_column_name: &str,
    ) -> Option<ColumnSchemaDescriptor> {
        self.columns
            .iter()
            .copied()
            .find(|column| column.source_column_name() == source_column_name)
    }

    #[must_use]
    pub fn column_by_source_column_crc(
        self,
        source_column_crc: u32,
    ) -> Option<ColumnSchemaDescriptor> {
        self.columns
            .iter()
            .copied()
            .find(|column| column.source_column_crc() == source_column_crc)
    }

    #[must_use]
    pub fn schema_hash(self) -> SchemaHash {
        row_schema_hash(&self)
    }

    /// Checks that this row schema can address a physical table unambiguously.
    ///
    /// # Errors
    ///
    /// Returns [`RowSchemaDescriptorError::EmptyName`] or
    /// [`RowSchemaDescriptorError::MissingColumns`] when the schema names
    /// nothing to bind to, [`RowSchemaDescriptorError::EmptyColumnField`] or
    /// [`RowSchemaDescriptorError::EmptySourceColumn`] for a column that names
    /// neither side of its mapping, and
    /// [`RowSchemaDescriptorError::DuplicateFieldName`] or
    /// [`RowSchemaDescriptorError::DuplicateSourceColumn`] when two columns
    /// would resolve to the same field or the same source-column CRC. Per
    /// column it also returns whatever the enum and foreign-key metadata
    /// checks reject, such as enum variants on a cell type that cannot carry
    /// them.
    pub fn validate(self) -> Result<(), RowSchemaDescriptorError> {
        if self.name.is_empty() {
            return Err(RowSchemaDescriptorError::EmptyName);
        }
        if self.columns.is_empty() {
            return Err(RowSchemaDescriptorError::MissingColumns { schema: self.name });
        }

        let mut fields = BTreeSet::new();
        let mut source_columns = BTreeSet::new();
        for column in self.columns {
            if column.field_name().is_empty() {
                return Err(RowSchemaDescriptorError::EmptyColumnField { schema: self.name });
            }
            if column.source_column_name().is_empty() {
                return Err(RowSchemaDescriptorError::EmptySourceColumn {
                    schema: self.name,
                    field: column.field_name(),
                });
            }
            if !fields.insert(column.field_name()) {
                return Err(RowSchemaDescriptorError::DuplicateFieldName {
                    schema: self.name,
                    field: column.field_name(),
                });
            }
            if !source_columns.insert(column.source_column_crc()) {
                return Err(RowSchemaDescriptorError::DuplicateSourceColumn {
                    schema: self.name,
                    source_column: column.source_column_name(),
                });
            }
            validate_column_metadata(self.name, *column)?;
        }
        Ok(())
    }
}

/// Static catalog of merged row schemas available to the source compiler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSchemaCatalog {
    schemas: &'static [RowSchemaDescriptor],
}

impl RowSchemaCatalog {
    #[inline]
    #[must_use]
    pub const fn new(schemas: &'static [RowSchemaDescriptor]) -> Self {
        Self { schemas }
    }

    #[inline]
    #[must_use]
    pub const fn schemas(self) -> &'static [RowSchemaDescriptor] {
        self.schemas
    }

    #[inline]
    #[must_use]
    pub const fn len(self) -> usize {
        self.schemas.len()
    }

    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.schemas.is_empty()
    }

    /// Every schema in the catalog as registry entries, ready for
    /// `Registrar::register_many`.
    pub fn rows(self) -> impl Iterator<Item = RowSchema> {
        self.schemas.iter().map(RowSchema::new)
    }

    #[must_use]
    pub fn by_name(self, name: &str) -> Option<&'static RowSchemaDescriptor> {
        self.schemas.iter().find(|schema| schema.name() == name)
    }

    #[must_use]
    pub fn by_crc(self, name_crc: u32) -> Option<&'static RowSchemaDescriptor> {
        self.schemas
            .iter()
            .find(|schema| schema.name_crc() == name_crc)
    }

    /// Checks every schema in the catalog and that they stay distinguishable.
    ///
    /// # Errors
    ///
    /// Returns whatever [`RowSchemaDescriptor::validate`] returns for the
    /// first invalid schema, then
    /// [`RowSchemaDescriptorError::DuplicateSchemaName`] or
    /// [`RowSchemaDescriptorError::DuplicateSchemaCrc`] when two catalog
    /// entries share a name or a lowercase name CRC and a compiled table could
    /// not tell them apart.
    pub fn validate(self) -> Result<(), RowSchemaDescriptorError> {
        let mut names = BTreeSet::new();
        let mut crcs = BTreeSet::new();
        for schema in self.schemas {
            schema.validate()?;
            if !names.insert(schema.name()) {
                return Err(RowSchemaDescriptorError::DuplicateSchemaName {
                    schema: schema.name(),
                });
            }
            if !crcs.insert(schema.name_crc()) {
                return Err(RowSchemaDescriptorError::DuplicateSchemaCrc {
                    schema: schema.name(),
                    schema_crc: schema.name_crc(),
                });
            }
        }
        Ok(())
    }
}

/// A physical authored table bound to its merged logical row schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthoredTableSchema<'a> {
    table_name: &'a str,
    row_schema: &'a RowSchemaDescriptor,
    key_field_name: Option<&'static str>,
}

impl<'a> AuthoredTableSchema<'a> {
    /// Binds one physical table name to a merged row schema and its key.
    ///
    /// # Errors
    ///
    /// Returns [`AuthoredTableSchemaError::UnknownKeyField`] when
    /// `key_field_name` names no column of `row_schema`,
    /// [`AuthoredTableSchemaError::NotKeyCandidate`] when the named column
    /// cannot carry a row key, and [`AuthoredTableSchemaError::MissingTableKey`]
    /// when no key was named at all yet the schema does offer a candidate.
    pub fn new(
        table_name: &'a str,
        row_schema: &'a RowSchemaDescriptor,
        key_field_name: Option<&str>,
    ) -> Result<Self, AuthoredTableSchemaError> {
        let key = key_field_name
            .map(|key_field_name| {
                row_schema
                    .column_by_field_name(key_field_name)
                    .ok_or_else(|| AuthoredTableSchemaError::UnknownKeyField {
                        table: table_name.to_owned(),
                        schema: row_schema.name(),
                        key: key_field_name.to_owned(),
                    })
            })
            .transpose()?;
        if let Some(key) = key {
            if !key.is_key_candidate() {
                return Err(AuthoredTableSchemaError::NotKeyCandidate {
                    table: table_name.to_owned(),
                    schema: row_schema.name(),
                    key: key.field_name().to_owned(),
                });
            }
        } else if let Some(candidate) = row_schema.key_candidates().next() {
            return Err(AuthoredTableSchemaError::MissingTableKey {
                table: table_name.to_owned(),
                schema: row_schema.name(),
                candidate: candidate.field_name(),
            });
        }
        Ok(Self {
            table_name,
            row_schema,
            key_field_name: key.map(ColumnSchemaDescriptor::field_name),
        })
    }

    #[inline]
    #[must_use]
    pub const fn name(self) -> &'a str {
        self.table_name
    }

    #[inline]
    #[must_use]
    pub const fn row(self) -> &'static str {
        self.row_schema.name()
    }

    #[inline]
    #[must_use]
    pub const fn crc(self) -> u32 {
        Crc32::from_str_lower(self.table_name).value()
    }

    #[inline]
    #[must_use]
    pub const fn row_crc(self) -> u32 {
        self.row_schema.name_crc()
    }

    #[inline]
    #[must_use]
    pub const fn row_schema(self) -> &'a RowSchemaDescriptor {
        self.row_schema
    }

    #[inline]
    #[must_use]
    pub const fn key_field_name(self) -> Option<&'static str> {
        self.key_field_name
    }

    #[inline]
    #[must_use]
    pub const fn column_count(self) -> usize {
        self.row_schema.column_count()
    }

    #[inline]
    #[must_use]
    pub fn column(self, index: usize) -> Option<ColumnSchema> {
        self.row_schema.column(index).map(move |column| {
            let row_key = Some(column.field_name()) == self.key_field_name;
            ColumnSchema::from_descriptor(column, row_key)
        })
    }

    #[must_use]
    pub fn columns(self) -> impl ExactSizeIterator<Item = ColumnSchema> + 'a {
        self.row_schema
            .columns()
            .iter()
            .copied()
            .map(move |column| {
                let row_key = Some(column.field_name()) == self.key_field_name;
                ColumnSchema::from_descriptor(column, row_key)
            })
    }

    #[must_use]
    pub fn has_column_crc(self, column_crc: u32) -> bool {
        self.columns()
            .any(|column| column.column_crc() == column_crc)
    }

    #[must_use]
    pub fn schema_hash(self) -> SchemaHash {
        self.row_schema.schema_hash()
    }
}

#[cfg(feature = "authoring")]
impl AuthoringTableSchema for AuthoredTableSchema<'_> {
    fn name(&self) -> &str {
        AuthoredTableSchema::name(*self)
    }

    fn crc(&self) -> u32 {
        AuthoredTableSchema::crc(*self)
    }

    fn row_crc(&self) -> u32 {
        AuthoredTableSchema::row_crc(*self)
    }

    fn column_count(&self) -> usize {
        AuthoredTableSchema::column_count(*self)
    }

    fn column(&self, index: usize) -> Option<ColumnSchema> {
        AuthoredTableSchema::column(*self, index)
    }

    fn columns(&self) -> AuthoringColumns<'_> {
        AuthoringColumns {
            table: self,
            next_index: 0,
        }
    }

    fn schema_hash(&self) -> SchemaHash {
        AuthoredTableSchema::schema_hash(*self)
    }
}

/// Registry entry for one merged logical row schema.
///
/// A contribution registers its whole [`RowSchemaCatalog`] at once through
/// [`RowSchemaCatalog::rows`]; the registry keys on the schema name, so two
/// contributions claiming the same row shape is a composition error rather than
/// a first-one-wins lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowSchema(&'static RowSchemaDescriptor);

impl RowSchema {
    #[inline]
    #[must_use]
    pub const fn new(descriptor: &'static RowSchemaDescriptor) -> Self {
        Self(descriptor)
    }

    /// The merged row schema this entry contributes.
    #[inline]
    #[must_use]
    pub const fn descriptor(self) -> &'static RowSchemaDescriptor {
        self.0
    }
}

impl az_gem_contract::RegistryEntry for RowSchema {
    type Key = &'static str;
    type Requires = az_gem_contract::Unconditional;

    fn registry_name() -> &'static str {
        "gamedata-row-schema"
    }

    fn key(&self) -> &'static str {
        self.0.name()
    }
}

/// Computes the stable schema hash shared by every table using this row shape.
#[must_use]
pub fn row_schema_hash(schema: &RowSchemaDescriptor) -> SchemaHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"azoth.gamedata.row-schema.v1\0");
    hash_str(&mut hasher, schema.name());
    hasher.update(&(schema.column_count() as u64).to_le_bytes());
    for field in schema.columns() {
        hash_str(&mut hasher, field.field_name());
        hash_str(&mut hasher, field.source_column_name());
        hash_cell_type(&mut hasher, field.cell_type());
        hasher.update(&[u8::from(field.is_key_candidate())]);
        hasher.update(&[u8::from(field.is_required())]);
        hasher.update(&(field.enum_variants().len() as u64).to_le_bytes());
        for variant in field.enum_variants() {
            hash_str(&mut hasher, variant.name());
            hasher.update(&variant.discriminant().to_le_bytes());
            hasher.update(&(variant.source_tokens().len() as u64).to_le_bytes());
            for token in variant.source_tokens() {
                hash_str(&mut hasher, token);
            }
        }
        hasher.update(&(field.foreign_key_targets().len() as u64).to_le_bytes());
        for fk in field.foreign_key_targets() {
            hasher.update(&fk.target_table_crc().to_le_bytes());
            hasher.update(&fk.target_row_crc().to_le_bytes());
            hasher.update(&fk.target_column_crc().to_le_bytes());
        }
    }
    // blake3 digests are a fixed 32 bytes, so the leading eight destructure out
    // instead of going through a fallible slice conversion.
    let [b0, b1, b2, b3, b4, b5, b6, b7, ..] = *hasher.finalize().as_bytes();
    SchemaHash(u64::from_le_bytes([b0, b1, b2, b3, b4, b5, b6, b7]))
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum RowSchemaDescriptorError {
    #[error("GameData row schema name is empty")]
    EmptyName,
    #[error("GameData row schema `{schema}` must declare at least one column")]
    MissingColumns { schema: &'static str },
    #[error("GameData row schema `{schema}` has a column with an empty field name")]
    EmptyColumnField { schema: &'static str },
    #[error("GameData row schema `{schema}` field `{field}` has an empty source column")]
    EmptySourceColumn {
        schema: &'static str,
        field: &'static str,
    },
    #[error("GameData row schema `{schema}` declares duplicate field `{field}`")]
    DuplicateFieldName {
        schema: &'static str,
        field: &'static str,
    },
    #[error("GameData row schema `{schema}` declares duplicate source column `{source_column}`")]
    DuplicateSourceColumn {
        schema: &'static str,
        source_column: &'static str,
    },
    #[error(
        "GameData row schema `{schema}` field `{field}` has enum metadata on unsupported cell type {cell_type:?}"
    )]
    EnumOnUnsupportedCellType {
        schema: &'static str,
        field: &'static str,
        cell_type: CellType,
    },
    #[error("GameData row schema `{schema}` field `{field}` declares an empty enum variant")]
    EmptyEnumVariant {
        schema: &'static str,
        field: &'static str,
    },
    #[error("GameData row schema `{schema}` field `{field}` declares an empty enum source token")]
    EmptyEnumSourceToken {
        schema: &'static str,
        field: &'static str,
    },
    #[error(
        "GameData row schema `{schema}` field `{field}` declares duplicate enum token `{token}`"
    )]
    DuplicateEnumToken {
        schema: &'static str,
        field: &'static str,
        token: &'static str,
    },
    #[error("GameData row schema `{schema}` field `{field}` has an invalid foreign-key target")]
    InvalidForeignKeyTarget {
        schema: &'static str,
        field: &'static str,
    },
    #[error("GameData row schema catalog declares duplicate schema `{schema}`")]
    DuplicateSchemaName { schema: &'static str },
    #[error("GameData row schema catalog has a CRC collision at `{schema}` ({schema_crc:#010x})")]
    DuplicateSchemaCrc {
        schema: &'static str,
        schema_crc: u32,
    },
}

#[derive(Debug, Error)]
pub enum AuthoredTableSchemaError {
    #[error(
        "GameData table `{table}` omits `key`, but row schema `{schema}` has key candidate `{candidate}`"
    )]
    MissingTableKey {
        table: String,
        schema: &'static str,
        candidate: &'static str,
    },
    #[error(
        "GameData table `{table}` selects unknown key field `{key}` from row schema `{schema}`"
    )]
    UnknownKeyField {
        table: String,
        schema: &'static str,
        key: String,
    },
    #[error(
        "GameData table `{table}` selects `{key}` as its key, but it is not a #[key] candidate in row schema `{schema}`"
    )]
    NotKeyCandidate {
        table: String,
        schema: &'static str,
        key: String,
    },
}

fn validate_column_metadata(
    schema: &'static str,
    column: ColumnSchemaDescriptor,
) -> Result<(), RowSchemaDescriptorError> {
    if !column.enum_variants().is_empty() && enum_representation_type(column.cell_type()).is_none()
    {
        return Err(RowSchemaDescriptorError::EnumOnUnsupportedCellType {
            schema,
            field: column.field_name(),
            cell_type: column.cell_type(),
        });
    }
    let mut enum_tokens = BTreeMap::new();
    for (variant_index, variant) in column.enum_variants().iter().enumerate() {
        if variant.name().is_empty() {
            return Err(RowSchemaDescriptorError::EmptyEnumVariant {
                schema,
                field: column.field_name(),
            });
        }
        for token in std::iter::once(variant.name()).chain(variant.source_tokens().iter().copied())
        {
            if token.is_empty() {
                return Err(RowSchemaDescriptorError::EmptyEnumSourceToken {
                    schema,
                    field: column.field_name(),
                });
            }
            let normalized = token.to_ascii_lowercase();
            if enum_tokens
                .insert(normalized, variant_index)
                .is_some_and(|owner| owner != variant_index)
            {
                return Err(RowSchemaDescriptorError::DuplicateEnumToken {
                    schema,
                    field: column.field_name(),
                    token,
                });
            }
        }
    }
    for target in column.foreign_key_targets() {
        if target.target_table().is_empty()
            || target.target_row().is_empty()
            || target.target_column().is_empty()
        {
            return Err(RowSchemaDescriptorError::InvalidForeignKeyTarget {
                schema,
                field: column.field_name(),
            });
        }
    }
    Ok(())
}

fn enum_representation_type(cell_type: CellType) -> Option<crate::table::ScalarType> {
    use crate::table::ScalarType;

    let scalar = match cell_type {
        CellType::Scalar(scalar) | CellType::List(ListElementType::Scalar(scalar)) => scalar,
        CellType::Range(_)
        | CellType::List(ListElementType::Range(_) | ListElementType::Pair(_)) => {
            return None;
        }
    };
    matches!(
        scalar,
        ScalarType::I8
            | ScalarType::I16
            | ScalarType::I32
            | ScalarType::I64
            | ScalarType::U8
            | ScalarType::U16
            | ScalarType::U32
            | ScalarType::U64
            | ScalarType::Crc32
            | ScalarType::String
    )
    .then_some(scalar)
}

fn hash_cell_type(hasher: &mut blake3::Hasher, cell_type: CellType) {
    match cell_type {
        CellType::Scalar(scalar) => {
            hasher.update(&[1, scalar.id()]);
        }
        CellType::Range(range) => {
            hasher.update(&[2]);
            hash_range_type(hasher, range);
        }
        CellType::List(element) => {
            hasher.update(&[3]);
            hash_list_element_type(hasher, element);
        }
    }
}

fn hash_list_element_type(hasher: &mut blake3::Hasher, element_type: ListElementType) {
    match element_type {
        ListElementType::Scalar(scalar) => {
            hasher.update(&[1, scalar.id()]);
        }
        ListElementType::Range(range) => {
            hasher.update(&[2]);
            hash_range_type(hasher, range);
        }
        ListElementType::Pair(pair) => {
            hasher.update(&[3]);
            hash_pair_type(hasher, pair);
        }
    }
}

fn hash_pair_type(hasher: &mut blake3::Hasher, pair: PairType) {
    hash_atom_type(hasher, pair.first);
    hash_atom_type(hasher, pair.second);
}

fn hash_range_type(hasher: &mut blake3::Hasher, range: RangeType) {
    hasher.update(&[range.bounds.id(), range.endpoint.id()]);
}

fn hash_atom_type(hasher: &mut blake3::Hasher, atom: AtomType) {
    match atom {
        AtomType::Scalar(scalar) => {
            hasher.update(&[1, scalar.id()]);
        }
        AtomType::Range(range) => {
            hasher.update(&[2]);
            hash_range_type(hasher, range);
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
    use crate::table::{ListElementType, RangeBounds, RangeEndpointType, RangeType, ScalarType};

    const ITEM_COLUMNS: &[ColumnSchemaDescriptor] = &[
        ColumnSchemaDescriptor::new("item_id", "ItemID", CellType::Scalar(ScalarType::RowKey))
            .key_candidate(true),
        ColumnSchemaDescriptor::new(
            "display_name",
            "DisplayName",
            CellType::Scalar(ScalarType::String),
        )
        .optional(),
    ];
    const ITEM_SCHEMA: RowSchemaDescriptor = RowSchemaDescriptor::new("ItemData", ITEM_COLUMNS);
    const CATALOG: RowSchemaCatalog = RowSchemaCatalog::new(&[ITEM_SCHEMA]);
    const SELF_ALIASED_VARIANTS: &[EnumVariantDescriptor] = &[
        EnumVariantDescriptor::new("None", &["None"], 0),
        EnumVariantDescriptor::new("Expansion2023", &["Expansion2023"], 1),
    ];
    const SELF_ALIASED_COLUMNS: &[ColumnSchemaDescriptor] =
        &[
            ColumnSchemaDescriptor::new("expansion", "Expansion", CellType::Scalar(ScalarType::U8))
                .with_enum_variants(SELF_ALIASED_VARIANTS),
        ];
    const SELF_ALIASED_SCHEMA: RowSchemaDescriptor =
        RowSchemaDescriptor::new("SelfAliased", SELF_ALIASED_COLUMNS);
    const COLLIDING_VARIANTS: &[EnumVariantDescriptor] = &[
        EnumVariantDescriptor::new("First", &["shared"], 0),
        EnumVariantDescriptor::new("Second", &["SHARED"], 1),
    ];
    const COLLIDING_COLUMNS: &[ColumnSchemaDescriptor] =
        &[
            ColumnSchemaDescriptor::new("value", "Value", CellType::Scalar(ScalarType::U8))
                .with_enum_variants(COLLIDING_VARIANTS),
        ];
    const COLLIDING_SCHEMA: RowSchemaDescriptor =
        RowSchemaDescriptor::new("Colliding", COLLIDING_COLUMNS);
    const ENUM_LIST_COLUMNS: &[ColumnSchemaDescriptor] = &[ColumnSchemaDescriptor::new(
        "values",
        "Values",
        CellType::List(ListElementType::Scalar(ScalarType::U8)),
    )
    .with_enum_variants(SELF_ALIASED_VARIANTS)];
    const ENUM_LIST_SCHEMA: RowSchemaDescriptor =
        RowSchemaDescriptor::new("EnumList", ENUM_LIST_COLUMNS);
    const ENUM_RANGE_LIST_COLUMNS: &[ColumnSchemaDescriptor] = &[ColumnSchemaDescriptor::new(
        "values",
        "Values",
        CellType::List(ListElementType::Range(RangeType::new(
            RangeBounds::Inclusive,
            RangeEndpointType::U32,
        ))),
    )
    .with_enum_variants(SELF_ALIASED_VARIANTS)];
    const ENUM_RANGE_LIST_SCHEMA: RowSchemaDescriptor =
        RowSchemaDescriptor::new("EnumRangeList", ENUM_RANGE_LIST_COLUMNS);
    const STRING_ENUM_COLUMNS: &[ColumnSchemaDescriptor] =
        &[
            ColumnSchemaDescriptor::new("value", "Value", CellType::Scalar(ScalarType::String))
                .with_enum_variants(SELF_ALIASED_VARIANTS),
        ];
    const STRING_ENUM_SCHEMA: RowSchemaDescriptor =
        RowSchemaDescriptor::new("StringEnum", STRING_ENUM_COLUMNS);

    #[test]
    fn catalog_resolves_one_merged_row_schema() {
        CATALOG.validate().expect("valid schema catalog");
        assert_eq!(CATALOG.by_name("ItemData"), Some(&ITEM_SCHEMA));
        assert_eq!(CATALOG.by_crc(ITEM_SCHEMA.name_crc()), Some(&ITEM_SCHEMA));
    }

    #[test]
    fn physical_tables_share_the_row_schema_hash() {
        let master =
            AuthoredTableSchema::new("MasterItemDefinitions", &ITEM_SCHEMA, Some("item_id"))
                .expect("valid table key");
        let events =
            AuthoredTableSchema::new("EventItemDefinitions", &ITEM_SCHEMA, Some("item_id"))
                .expect("valid table key");
        assert_ne!(master.crc(), events.crc());
        assert_eq!(master.row_crc(), events.row_crc());
        assert_eq!(master.schema_hash(), events.schema_hash());
    }

    #[test]
    fn enum_source_token_may_repeat_its_own_canonical_name() {
        SELF_ALIASED_SCHEMA
            .validate()
            .expect("a source token may preserve its own canonical spelling");
    }

    #[test]
    fn enum_tokens_must_remain_unique_across_variants() {
        let error = COLLIDING_SCHEMA
            .validate()
            .expect_err("case-insensitive cross-variant aliases must remain ambiguous");

        assert!(matches!(
            error,
            RowSchemaDescriptorError::DuplicateEnumToken {
                schema: "Colliding",
                field: "value",
                token: "SHARED",
            }
        ));
    }

    #[test]
    fn enum_metadata_applies_to_scalar_list_elements() {
        ENUM_LIST_SCHEMA
            .validate()
            .expect("enum metadata may type each scalar element of a list");
    }

    #[test]
    fn enum_metadata_rejects_non_scalar_list_elements() {
        let error = ENUM_RANGE_LIST_SCHEMA
            .validate()
            .expect_err("range list elements cannot represent enum discriminants");

        assert!(matches!(
            error,
            RowSchemaDescriptorError::EnumOnUnsupportedCellType {
                schema: "EnumRangeList",
                field: "values",
                ..
            }
        ));
    }

    #[test]
    fn enum_metadata_may_describe_known_values_of_an_open_string_field() {
        STRING_ENUM_SCHEMA
            .validate()
            .expect("string-backed enums preserve their open source representation");
    }
}
