//! Generated authored-schema lock model and drift classification.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};

pub const AUTHORED_SCHEMA_LOCK_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthoredSchemaLock {
    pub format_version: u32,
    pub schemas: Vec<LockedSchema>,
}

impl AuthoredSchemaLock {
    #[must_use]
    pub fn new(mut schemas: Vec<LockedSchema>) -> Self {
        schemas.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            format_version: AUTHORED_SCHEMA_LOCK_FORMAT_VERSION,
            schemas,
        }
    }

    #[must_use]
    pub fn schema(&self, name: &str) -> Option<&LockedSchema> {
        self.schemas.iter().find(|schema| schema.name == name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedSchema {
    pub name: String,
    pub version: u32,
    pub shape: LockedSchemaShape,
    pub fields: Vec<LockedField>,
    pub variants: Vec<LockedVariant>,
    pub tombstoned_field_ids: Vec<u32>,
    pub tombstoned_variant_ids: Vec<u32>,
}

impl LockedSchema {
    #[must_use]
    pub fn new(name: impl Into<String>, version: u32, shape: LockedSchemaShape) -> Self {
        Self {
            name: name.into(),
            version,
            shape,
            fields: Vec::new(),
            variants: Vec::new(),
            tombstoned_field_ids: Vec::new(),
            tombstoned_variant_ids: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_fields(mut self, mut fields: Vec<LockedField>) -> Self {
        fields.sort_by_key(|field| field.id);
        self.fields = fields;
        self
    }

    #[must_use]
    pub fn with_variants(mut self, mut variants: Vec<LockedVariant>) -> Self {
        variants.sort_by_key(|variant| variant.id);
        self.variants = variants;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockedSchemaShape {
    Struct,
    UnitStruct,
    Enum,
    Primitive {
        kind: String,
    },
    List {
        element_type: String,
    },
    Optional {
        value_type: String,
    },
    Map {
        key_type: String,
        value_type: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedField {
    pub id: u32,
    pub name: String,
    pub schema_type: String,
}

impl LockedField {
    #[must_use]
    pub fn new(id: u32, name: impl Into<String>, schema_type: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            schema_type: schema_type.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LockedVariant {
    pub id: u32,
    pub name: String,
    pub payload_schema_type: Option<String>,
}

impl LockedVariant {
    #[must_use]
    pub fn new(
        id: u32,
        name: impl Into<String>,
        payload_schema_type: Option<impl Into<String>>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            payload_schema_type: payload_schema_type.map(Into::into),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaMigrationEdge {
    pub schema: String,
    pub from_version: u32,
    pub to_version: u32,
}

impl SchemaMigrationEdge {
    #[must_use]
    pub fn new(schema: impl Into<String>, from_version: u32, to_version: u32) -> Self {
        Self {
            schema: schema.into(),
            from_version,
            to_version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedSchemaChange {
    pub schema: String,
    pub class: SchemaChangeClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaChangeClass {
    Compatible(CompatibleSchemaChange),
    Breaking(BreakingSchemaChange),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibleSchemaChange {
    OptionalFieldAdded { id: u32 },
    FieldRenamed { id: u32 },
    VariantAdded { id: u32 },
    VariantRenamed { id: u32 },
    SchemaAdded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BreakingSchemaChange {
    SchemaRemoved,
    SchemaShapeChanged,
    FieldRemoved { id: u32 },
    FieldTombstoned { id: u32 },
    RequiredFieldAdded { id: u32 },
    FieldTypeChanged { id: u32 },
    FieldIdReused { id: u32 },
    FieldTombstoneRemoved { id: u32 },
    VariantRemoved { id: u32 },
    VariantTombstoned { id: u32 },
    VariantPayloadChanged { id: u32 },
    VariantIdReused { id: u32 },
    VariantTombstoneRemoved { id: u32 },
    VersionDecreased,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaLockViolation {
    pub schema: String,
    pub locked_version: u32,
    pub current_version: u32,
    pub breaking_changes: Vec<BreakingSchemaChange>,
    pub reason: SchemaLockViolationReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaLockViolationReason {
    VersionNotIncremented,
    MigrationChainMissing,
}

/// Classifies descriptor drift without applying version/migration policy.
#[must_use]
pub fn classify_authored_schema_drift(
    locked: &AuthoredSchemaLock,
    current: &AuthoredSchemaLock,
) -> Vec<ClassifiedSchemaChange> {
    let mut changes = Vec::new();
    let locked_by_name = locked
        .schemas
        .iter()
        .map(|schema| (schema.name.as_str(), schema))
        .collect::<BTreeMap<_, _>>();
    let current_by_name = current
        .schemas
        .iter()
        .map(|schema| (schema.name.as_str(), schema))
        .collect::<BTreeMap<_, _>>();

    for schema in &locked.schemas {
        let Some(current_schema) = current_by_name.get(schema.name.as_str()).copied() else {
            changes.push(change(&schema.name, BreakingSchemaChange::SchemaRemoved));
            continue;
        };
        classify_schema(schema, current_schema, current, &mut changes);
    }
    for schema in &current.schemas {
        if !locked_by_name.contains_key(schema.name.as_str()) {
            changes.push(compatible(
                &schema.name,
                CompatibleSchemaChange::SchemaAdded,
            ));
        }
    }
    changes
}

fn classify_schema(
    locked: &LockedSchema,
    current: &LockedSchema,
    current_lock: &AuthoredSchemaLock,
    changes: &mut Vec<ClassifiedSchemaChange>,
) {
    if locked.shape != current.shape {
        changes.push(change(
            &locked.name,
            BreakingSchemaChange::SchemaShapeChanged,
        ));
    }
    if current.version < locked.version {
        changes.push(change(&locked.name, BreakingSchemaChange::VersionDecreased));
    }
    classify_fields(locked, current, current_lock, changes);
    classify_variants(locked, current, changes);
}

fn classify_fields(
    locked: &LockedSchema,
    current: &LockedSchema,
    current_lock: &AuthoredSchemaLock,
    changes: &mut Vec<ClassifiedSchemaChange>,
) {
    let old = locked
        .fields
        .iter()
        .map(|field| (field.id, field))
        .collect::<BTreeMap<_, _>>();
    let new = current
        .fields
        .iter()
        .map(|field| (field.id, field))
        .collect::<BTreeMap<_, _>>();
    let old_tombstones = locked
        .tombstoned_field_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let new_tombstones = current
        .tombstoned_field_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for field in &locked.fields {
        match new.get(&field.id) {
            None if new_tombstones.contains(&field.id) => changes.push(change(
                &locked.name,
                BreakingSchemaChange::FieldTombstoned { id: field.id },
            )),
            None => changes.push(change(
                &locked.name,
                BreakingSchemaChange::FieldRemoved { id: field.id },
            )),
            Some(current_field) if current_field.schema_type != field.schema_type => {
                changes.push(change(
                    &locked.name,
                    BreakingSchemaChange::FieldTypeChanged { id: field.id },
                ));
            }
            Some(current_field) if current_field.name != field.name => changes.push(compatible(
                &locked.name,
                CompatibleSchemaChange::FieldRenamed { id: field.id },
            )),
            Some(_) => {}
        }
    }
    for field in &current.fields {
        if old.contains_key(&field.id) {
            continue;
        }
        if old_tombstones.contains(&field.id) {
            changes.push(change(
                &locked.name,
                BreakingSchemaChange::FieldIdReused { id: field.id },
            ));
        } else if current_lock
            .schema(&field.schema_type)
            .is_some_and(|schema| matches!(schema.shape, LockedSchemaShape::Optional { .. }))
        {
            changes.push(compatible(
                &locked.name,
                CompatibleSchemaChange::OptionalFieldAdded { id: field.id },
            ));
        } else {
            changes.push(change(
                &locked.name,
                BreakingSchemaChange::RequiredFieldAdded { id: field.id },
            ));
        }
    }
    for id in old_tombstones {
        if !new_tombstones.contains(&id) && !new.contains_key(&id) {
            changes.push(change(
                &locked.name,
                BreakingSchemaChange::FieldTombstoneRemoved { id },
            ));
        }
    }
}

fn classify_variants(
    locked: &LockedSchema,
    current: &LockedSchema,
    changes: &mut Vec<ClassifiedSchemaChange>,
) {
    let old = locked
        .variants
        .iter()
        .map(|variant| (variant.id, variant))
        .collect::<BTreeMap<_, _>>();
    let new = current
        .variants
        .iter()
        .map(|variant| (variant.id, variant))
        .collect::<BTreeMap<_, _>>();
    let old_tombstones = locked
        .tombstoned_variant_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let new_tombstones = current
        .tombstoned_variant_ids
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for variant in &locked.variants {
        match new.get(&variant.id) {
            None if new_tombstones.contains(&variant.id) => changes.push(change(
                &locked.name,
                BreakingSchemaChange::VariantTombstoned { id: variant.id },
            )),
            None => changes.push(change(
                &locked.name,
                BreakingSchemaChange::VariantRemoved { id: variant.id },
            )),
            Some(current_variant)
                if current_variant.payload_schema_type != variant.payload_schema_type =>
            {
                changes.push(change(
                    &locked.name,
                    BreakingSchemaChange::VariantPayloadChanged { id: variant.id },
                ));
            }
            Some(current_variant) if current_variant.name != variant.name => {
                changes.push(compatible(
                    &locked.name,
                    CompatibleSchemaChange::VariantRenamed { id: variant.id },
                ));
            }
            Some(_) => {}
        }
    }
    for variant in &current.variants {
        if old.contains_key(&variant.id) {
            continue;
        }
        if old_tombstones.contains(&variant.id) {
            changes.push(change(
                &locked.name,
                BreakingSchemaChange::VariantIdReused { id: variant.id },
            ));
        } else {
            changes.push(compatible(
                &locked.name,
                CompatibleSchemaChange::VariantAdded { id: variant.id },
            ));
        }
    }
    for id in old_tombstones {
        if !new_tombstones.contains(&id) && !new.contains_key(&id) {
            changes.push(change(
                &locked.name,
                BreakingSchemaChange::VariantTombstoneRemoved { id },
            ));
        }
    }
}

fn change(schema: &str, change: BreakingSchemaChange) -> ClassifiedSchemaChange {
    ClassifiedSchemaChange {
        schema: schema.to_string(),
        class: SchemaChangeClass::Breaking(change),
    }
}

fn compatible(schema: &str, change: CompatibleSchemaChange) -> ClassifiedSchemaChange {
    ClassifiedSchemaChange {
        schema: schema.to_string(),
        class: SchemaChangeClass::Compatible(change),
    }
}

/// Applies the lock policy to all breaking changes.
#[must_use]
pub fn authored_schema_lock_violations(
    locked: &AuthoredSchemaLock,
    current: &AuthoredSchemaLock,
    migrations: &[SchemaMigrationEdge],
) -> Vec<SchemaLockViolation> {
    let mut breaking = BTreeMap::<String, Vec<BreakingSchemaChange>>::new();
    for change in classify_authored_schema_drift(locked, current) {
        if let SchemaChangeClass::Breaking(change_class) = change.class {
            breaking
                .entry(change.schema)
                .or_default()
                .push(change_class);
        }
    }

    let mut violations = Vec::new();
    for (schema_name, breaking_changes) in breaking {
        let locked_version = locked
            .schema(&schema_name)
            .map_or(0, |schema| schema.version);
        let current_version = current
            .schema(&schema_name)
            .map_or(0, |schema| schema.version);
        let reason = if current_version <= locked_version {
            Some(SchemaLockViolationReason::VersionNotIncremented)
        } else if !migration_chain_exists(&schema_name, locked_version, current_version, migrations)
        {
            Some(SchemaLockViolationReason::MigrationChainMissing)
        } else {
            None
        };
        if let Some(reason) = reason {
            violations.push(SchemaLockViolation {
                schema: schema_name,
                locked_version,
                current_version,
                breaking_changes,
                reason,
            });
        }
    }
    violations
}

fn migration_chain_exists(
    schema: &str,
    from_version: u32,
    to_version: u32,
    migrations: &[SchemaMigrationEdge],
) -> bool {
    if from_version == to_version {
        return true;
    }
    let mut queue = VecDeque::from([from_version]);
    let mut seen = BTreeSet::from([from_version]);
    while let Some(version) = queue.pop_front() {
        for edge in migrations.iter().filter(|edge| {
            edge.schema == schema
                && edge.from_version == version
                && edge.to_version > version
                && edge.to_version <= to_version
        }) {
            if edge.to_version == to_version {
                return true;
            }
            if seen.insert(edge.to_version) {
                queue.push_back(edge.to_version);
            }
        }
    }
    false
}

/// Carries removed member IDs into generated tombstones and preserves all
/// prior tombstones, preventing future ID reuse.
#[must_use]
pub fn carry_forward_schema_tombstones(
    previous: &AuthoredSchemaLock,
    mut current: AuthoredSchemaLock,
) -> AuthoredSchemaLock {
    for schema in &mut current.schemas {
        let Some(old) = previous.schema(&schema.name) else {
            continue;
        };
        let current_fields = schema
            .fields
            .iter()
            .map(|field| field.id)
            .collect::<BTreeSet<_>>();
        let current_variants = schema
            .variants
            .iter()
            .map(|variant| variant.id)
            .collect::<BTreeSet<_>>();
        let mut field_tombstones = old
            .tombstoned_field_ids
            .iter()
            .copied()
            .chain(
                old.fields
                    .iter()
                    .map(|field| field.id)
                    .filter(|id| !current_fields.contains(id)),
            )
            .collect::<BTreeSet<_>>();
        let mut variant_tombstones = old
            .tombstoned_variant_ids
            .iter()
            .copied()
            .chain(
                old.variants
                    .iter()
                    .map(|variant| variant.id)
                    .filter(|id| !current_variants.contains(id)),
            )
            .collect::<BTreeSet<_>>();
        field_tombstones.retain(|id| !current_fields.contains(id));
        variant_tombstones.retain(|id| !current_variants.contains(id));
        schema.tombstoned_field_ids = field_tombstones.into_iter().collect();
        schema.tombstoned_variant_ids = variant_tombstones.into_iter().collect();
    }
    current
}

/// Decodes the generated RON lock.
///
/// # Errors
///
/// [`ron::error::SpannedError`] when `text` is not a well-formed
/// `AuthoredSchemaLock` document.
pub fn decode_authored_schema_lock(
    text: &str,
) -> Result<AuthoredSchemaLock, ron::error::SpannedError> {
    ron::from_str(text)
}

/// Encodes the generated RON lock deterministically.
///
/// # Errors
///
/// [`ron::Error`] when the lock cannot be serialised.
pub fn encode_authored_schema_lock(lock: &AuthoredSchemaLock) -> Result<String, ron::Error> {
    let mut text = ron::ser::to_string_pretty(lock, ron::ser::PrettyConfig::default())?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = "az.test.Root";
    const OPTIONAL: &str = "core.option<core.string>";

    fn lock(root: LockedSchema) -> AuthoredSchemaLock {
        AuthoredSchemaLock::new(vec![
            root,
            LockedSchema::new(
                OPTIONAL,
                1,
                LockedSchemaShape::Optional {
                    value_type: "core.string".to_string(),
                },
            ),
        ])
    }

    fn root_field(name: &str, schema_type: &str) -> LockedSchema {
        LockedSchema::new(ROOT, 1, LockedSchemaShape::Struct).with_fields(vec![LockedField::new(
            1,
            name,
            schema_type,
        )])
    }

    fn classes(old: &AuthoredSchemaLock, current: &AuthoredSchemaLock) -> Vec<SchemaChangeClass> {
        classify_authored_schema_drift(old, current)
            .into_iter()
            .filter(|change| change.schema == ROOT)
            .map(|change| change.class)
            .collect()
    }

    #[test]
    fn additive_optional_field_is_compatible() {
        let old = lock(root_field("name", "core.string"));
        let current = lock(root_field("name", "core.string").with_fields(vec![
            LockedField::new(1, "name", "core.string"),
            LockedField::new(2, "description", OPTIONAL),
        ]));

        assert!(
            classes(&old, &current).contains(&SchemaChangeClass::Compatible(
                CompatibleSchemaChange::OptionalFieldAdded { id: 2 }
            ))
        );
    }

    #[test]
    fn rename_with_retained_key_is_compatible() {
        let old = lock(root_field("old_name", "core.string"));
        let current = lock(root_field("new_name", "core.string"));

        assert_eq!(
            classes(&old, &current),
            vec![SchemaChangeClass::Compatible(
                CompatibleSchemaChange::FieldRenamed { id: 1 }
            )]
        );
    }

    #[test]
    fn removed_and_tombstoned_fields_are_breaking() {
        let old = lock(root_field("name", "core.string"));
        let removed = lock(LockedSchema::new(ROOT, 1, LockedSchemaShape::Struct));
        let mut tombstoned = LockedSchema::new(ROOT, 1, LockedSchemaShape::Struct);
        tombstoned.tombstoned_field_ids = vec![1];
        let tombstoned = lock(tombstoned);

        assert_eq!(
            classes(&old, &removed),
            vec![SchemaChangeClass::Breaking(
                BreakingSchemaChange::FieldRemoved { id: 1 }
            )]
        );
        assert_eq!(
            classes(&old, &tombstoned),
            vec![SchemaChangeClass::Breaking(
                BreakingSchemaChange::FieldTombstoned { id: 1 }
            )]
        );
    }

    #[test]
    fn field_type_change_is_breaking() {
        let old = lock(root_field("value", "core.string"));
        let current = lock(root_field("value", "core.u32"));

        assert_eq!(
            classes(&old, &current),
            vec![SchemaChangeClass::Breaking(
                BreakingSchemaChange::FieldTypeChanged { id: 1 }
            )]
        );
    }

    #[test]
    fn variant_removal_is_breaking() {
        let old = lock(
            LockedSchema::new(ROOT, 1, LockedSchemaShape::Enum)
                .with_variants(vec![LockedVariant::new(7, "Ready", None::<String>)]),
        );
        let current = lock(LockedSchema::new(ROOT, 1, LockedSchemaShape::Enum));

        assert_eq!(
            classes(&old, &current),
            vec![SchemaChangeClass::Breaking(
                BreakingSchemaChange::VariantRemoved { id: 7 }
            )]
        );
    }

    #[test]
    fn tombstoned_id_reuse_is_breaking() {
        let mut old_root = LockedSchema::new(ROOT, 1, LockedSchemaShape::Struct);
        old_root.tombstoned_field_ids = vec![9];
        let old = lock(old_root);
        let current = lock(
            LockedSchema::new(ROOT, 1, LockedSchemaShape::Struct)
                .with_fields(vec![LockedField::new(9, "reused", "core.string")]),
        );

        assert!(
            classes(&old, &current).contains(&SchemaChangeClass::Breaking(
                BreakingSchemaChange::FieldIdReused { id: 9 }
            ))
        );
    }

    #[test]
    fn breaking_change_requires_version_increment_and_migration_chain() {
        let old = lock(root_field("value", "core.string"));
        let mut current_root = root_field("value", "core.u32");
        current_root.version = 3;
        let current = lock(current_root);

        assert_eq!(
            authored_schema_lock_violations(&old, &current, &[])[0].reason,
            SchemaLockViolationReason::MigrationChainMissing
        );
        assert!(
            authored_schema_lock_violations(
                &old,
                &current,
                &[
                    SchemaMigrationEdge::new(ROOT, 1, 2),
                    SchemaMigrationEdge::new(ROOT, 2, 3),
                ],
            )
            .is_empty()
        );
    }
}
