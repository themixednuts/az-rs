//! Canonical sparse RON codec for [`PrefabDocument`](crate::PrefabDocument).

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use az_core::EntityId;
use bevy_reflect::{
    PartialReflect, ReflectDeserialize, ReflectFromReflect, ReflectRef, ReflectSerialize, TypeInfo,
    TypeRegistration, TypeRegistry,
    enums::{
        DynamicEnum, DynamicVariant, Enum as ReflectEnum, EnumInfo, StructVariantInfo, VariantInfo,
    },
    serde::{
        ReflectDeserializerProcessor, ReflectSerializerProcessor, TypedReflectDeserializer,
        TypedReflectSerializer,
    },
    structs::{DynamicStruct, Struct as ReflectStruct, StructInfo},
    tuple::DynamicTuple,
};
use ron::{ser::PrettyConfig, value::RawValue};
use serde::{
    Deserialize, Serialize,
    de::{DeserializeSeed, EnumAccess, Error as _, MapAccess, SeqAccess, VariantAccess, Visitor},
    ser::{Error as _, SerializeStruct, SerializeStructVariant},
};
use thiserror::Error;

use crate::{
    document::{
        EntityAlias, InstanceAlias, OverrideAction, OverrideOperation, OverrideTarget,
        PREFAB_DOCUMENT_VERSION, PrefabAssetPath, PrefabCatalogAlias, PrefabDocument,
        PrefabDocumentError, PrefabEntity, PrefabInstance, ReflectedPath, SparseValue,
    },
    migration::{PrefabMigrationError, PrefabRegistry},
    type_data::PrefabTypeData,
};

/// Registry-bound codec for canonical typed Prefab source.
pub struct PrefabCodec<'a> {
    registry: PrefabRegistry<'a>,
}

impl<'a> PrefabCodec<'a> {
    /// Binds the codec to one Bevy type registry, indexing its Prefab tags up
    /// front.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCodecError::Migration`] wrapping any error from
    /// `PrefabRegistry::try_new`, such as a colliding tag or a malformed
    /// migration graph.
    pub fn new(registry: &'a TypeRegistry) -> Result<Self, PrefabCodecError> {
        Ok(Self {
            registry: PrefabRegistry::try_new(registry)?,
        })
    }

    #[must_use]
    pub const fn registry(&self) -> &PrefabRegistry<'a> {
        &self.registry
    }

    /// Decodes, migrates, and canonicalizes one source document.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCodecError::Parse`] for malformed RON,
    /// [`PrefabCodecError::UnsupportedDocumentVersion`] for a container version
    /// this build does not speak, [`PrefabCodecError::MissingTypeVersion`] and
    /// [`PrefabCodecError::UnusedTypeVersion`] when the `type_versions` table
    /// does not match the components actually authored,
    /// [`PrefabCodecError::DuplicateCanonicalComponent`] when two encoded tags
    /// migrate onto one canonical component,
    /// [`PrefabCodecError::ReflectDeserialize`] and
    /// [`PrefabCodecError::InvalidReflectedShape`] for sparse values that do
    /// not fit their registered type, [`PrefabCodecError::Migration`] for tag
    /// resolution and migration failures, and [`PrefabCodecError::Document`]
    /// for invalid aliases, invalid source paths, or duplicate catalog
    /// aliases.
    pub fn decode(&self, source: &str) -> Result<PrefabDocument, PrefabCodecError> {
        let raw: RawDocument =
            ron::from_str(source).map_err(|error| PrefabCodecError::parse(&error))?;
        if raw.version != PREFAB_DOCUMENT_VERSION {
            return Err(PrefabCodecError::UnsupportedDocumentVersion {
                actual: raw.version,
                supported: PREFAB_DOCUMENT_VERSION,
            });
        }
        validate_declared_tags(&self.registry, &raw.type_versions.0)?;

        let mut used_encoded_tags = BTreeSet::new();
        let mut canonical_versions = BTreeMap::new();
        let mut entities = Vec::with_capacity(raw.entities.0.len());
        for (alias, raw_entity) in raw.entities.0 {
            let alias = EntityAlias::new(alias)?;
            let parent = raw_entity.parent.map(EntityAlias::new).transpose()?;
            let mut components = BTreeMap::new();
            for (encoded_tag, raw_value) in raw_entity.components.0 {
                used_encoded_tags.insert(encoded_tag.clone());
                let encoded_version = raw
                    .type_versions
                    .0
                    .get(&encoded_tag)
                    .copied()
                    .ok_or_else(|| PrefabCodecError::MissingTypeVersion(encoded_tag.clone()))?;
                let resolved = self.registry.resolve_tag(&encoded_tag)?;
                let sparse = deserialize_sparse(
                    &raw_value,
                    resolved.source_registration,
                    self.registry.type_registry(),
                )?;
                let sparse =
                    SparseValue::for_type(sparse, resolved.source_registration.type_info())?;
                let (canonical_tag, current_version, sparse) =
                    self.registry
                        .migrate(&encoded_tag, encoded_version, sparse)?;
                validate_sparse_shape(resolved.source_registration.type_info(), sparse.value())?;
                canonical_versions.insert(canonical_tag.clone(), current_version);
                if components.insert(canonical_tag.clone(), sparse).is_some() {
                    return Err(PrefabCodecError::DuplicateCanonicalComponent {
                        entity: alias.to_string(),
                        tag: canonical_tag,
                    });
                }
            }
            entities.push((
                alias,
                PrefabEntity {
                    entity_id: raw_entity.entity_id.map(EntityId::new),
                    parent,
                    components,
                },
            ));
        }

        let mut instances = Vec::with_capacity(raw.instances.0.len());
        for (alias, raw_instance) in raw.instances.0 {
            let alias = InstanceAlias::new(alias)?;
            let source = PrefabAssetPath::new(raw_instance.source)?;
            let parent = raw_instance.parent.map(EntityAlias::new).transpose()?;
            let overrides = raw_instance
                .overrides
                .into_iter()
                .map(|operation| self.decode_override(operation))
                .collect::<Result<Vec<_>, _>>()?;
            instances.push((
                alias,
                PrefabInstance {
                    source,
                    parent,
                    overrides,
                },
            ));
        }

        if let Some(unused) = raw
            .type_versions
            .0
            .keys()
            .find(|tag| !used_encoded_tags.contains(*tag))
        {
            return Err(PrefabCodecError::UnusedTypeVersion(unused.clone()));
        }

        let mut catalog_aliases = BTreeSet::new();
        for raw_alias in raw.catalog_aliases {
            let alias = PrefabCatalogAlias::new(raw_alias)?;
            if !catalog_aliases.insert(alias.clone()) {
                return Err(PrefabDocumentError::DuplicateAlias {
                    kind: "catalog",
                    alias: alias.to_string(),
                }
                .into());
            }
        }

        let mut document =
            PrefabDocument::try_from_parts(raw.version, canonical_versions, entities, instances)?;
        document.catalog_aliases = catalog_aliases;
        Ok(document)
    }

    /// Decodes one sparse typed-RON value carried by the project-host RPC
    /// envelope. The payload is a transport encoding, not a source document.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCodecError::ReflectDeserialize`] when `payload` is not
    /// UTF-8, [`PrefabCodecError::Parse`] when it is not valid RON,
    /// [`PrefabCodecError::Migration`] when `type_path` is unregistered,
    /// [`PrefabCodecError::InvalidReflectedShape`] when the decoded value does
    /// not fit the registered type, and [`PrefabCodecError::Document`] when it
    /// represents some other type.
    pub fn decode_sparse_value(
        &self,
        type_path: &str,
        payload: &[u8],
    ) -> Result<SparseValue, PrefabCodecError> {
        let source =
            std::str::from_utf8(payload).map_err(|error| PrefabCodecError::ReflectDeserialize {
                type_path: type_path.to_owned(),
                message: error.to_string(),
            })?;
        let raw = RawValue::from_ron(source).map_err(|error| PrefabCodecError::parse(&error))?;
        let registration = self.registry.resolve_type_path(type_path)?;
        let value = deserialize_sparse(raw, registration, self.registry.type_registry())?;
        validate_sparse_shape(registration.type_info(), value.as_ref())?;
        Ok(SparseValue::for_type(value, registration.type_info())?)
    }

    /// Encodes one sparse value as the canonical typed-RON RPC payload.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCodecError::ReflectSerialize`] when reflected
    /// serialization of `value` fails, for example because a retained field
    /// has no serializable reflect adapter.
    pub fn encode_sparse_value(&self, value: &SparseValue) -> Result<Vec<u8>, PrefabCodecError> {
        Ok(serialize_sparse(value, self.registry.type_registry())?
            .get_ron()
            .as_bytes()
            .to_vec())
    }

    /// Writes deterministic canonical source. Only fields retained in each
    /// [`SparseValue`] are emitted.
    ///
    /// # Errors
    ///
    /// Returns [`PrefabCodecError::UnsupportedDocumentVersion`],
    /// [`PrefabCodecError::NonCanonicalTag`],
    /// [`PrefabCodecError::NonCanonicalTypeVersion`],
    /// [`PrefabCodecError::MissingTypeVersion`],
    /// [`PrefabCodecError::UnusedTypeVersion`], or
    /// [`PrefabCodecError::InvalidReflectedShape`] when `document` is not in
    /// canonical form, plus [`PrefabCodecError::Document`] for a component
    /// value of the wrong type or an empty entity alias;
    /// [`PrefabCodecError::Migration`] for unresolvable tags and override
    /// component paths; [`PrefabCodecError::NotPrefabComponent`],
    /// [`PrefabCodecError::NotACollection`],
    /// [`PrefabCodecError::UnregisteredOverrideValue`], and
    /// [`PrefabCodecError::InvalidOverridePath`] for invalid overrides; and
    /// [`PrefabCodecError::ReflectSerialize`] or
    /// [`PrefabCodecError::Serialize`] when writing the RON itself fails.
    pub fn encode(&self, document: &PrefabDocument) -> Result<String, PrefabCodecError> {
        self.validate_canonical_document(document)?;
        let mut raw_entities = BTreeMap::new();
        for (alias, entity) in &document.entities {
            let mut components = BTreeMap::new();
            for (tag, value) in &entity.components {
                components.insert(
                    tag.clone(),
                    serialize_sparse(value, self.registry.type_registry())?,
                );
            }
            raw_entities.insert(
                alias.as_str().to_owned(),
                RawEntity {
                    entity_id: entity.entity_id.map(EntityId::value),
                    parent: entity
                        .parent
                        .as_ref()
                        .map(|parent| parent.as_str().to_owned()),
                    components: UniqueMap(components),
                },
            );
        }

        let mut raw_instances = BTreeMap::new();
        for (alias, instance) in &document.instances {
            let overrides = instance
                .overrides
                .iter()
                .map(|operation| self.encode_override(operation))
                .collect::<Result<Vec<_>, _>>()?;
            raw_instances.insert(
                alias.as_str().to_owned(),
                RawInstance {
                    source: instance.source.as_str().to_owned(),
                    parent: instance
                        .parent
                        .as_ref()
                        .map(|parent| parent.as_str().to_owned()),
                    overrides,
                },
            );
        }

        let raw = RawDocument {
            version: document.version,
            type_versions: UniqueMap(document.type_versions.clone()),
            catalog_aliases: document
                .catalog_aliases
                .iter()
                .map(|alias| alias.as_str().to_owned())
                .collect(),
            entities: UniqueMap(raw_entities),
            instances: UniqueMap(raw_instances),
        };
        let mut source = ron::ser::to_string_pretty(&raw, PrettyConfig::default())
            .map_err(|error| PrefabCodecError::serialize(&error))?;
        source.push('\n');
        Ok(source)
    }

    fn decode_override(
        &self,
        raw: RawOverrideOperation,
    ) -> Result<OverrideOperation, PrefabCodecError> {
        let target = decode_target(raw.target)?;
        let resolved = self
            .registry
            .resolve_prefab_type_path(&target.component)
            .map_err(|error| match error {
                PrefabMigrationError::MissingPrefabTypeData { .. } => {
                    PrefabCodecError::NotPrefabComponent(target.component.clone())
                }
                other => PrefabCodecError::Migration(other),
            })?;
        let target_type = type_at_path(
            resolved.source_registration.type_info(),
            target.path.segments(),
            &target.component,
        )?;
        let action = match raw.action {
            RawOverrideAction::Set(value) => {
                OverrideAction::Set(self.decode_override_value(&value, target_type, &target)?)
            }
            RawOverrideAction::Clear => OverrideAction::Clear,
            RawOverrideAction::Insert { index, value } => {
                let element_type = collection_element(target_type, &target)?;
                OverrideAction::Insert {
                    index,
                    value: self.decode_override_value(&value, element_type, &target)?,
                }
            }
            RawOverrideAction::Remove { index } => {
                collection_element(target_type, &target)?;
                OverrideAction::Remove { index }
            }
            RawOverrideAction::Move { from, to } => {
                collection_element(target_type, &target)?;
                OverrideAction::Move { from, to }
            }
        };
        Ok(OverrideOperation { target, action })
    }

    fn decode_override_value(
        &self,
        raw: &RawValue,
        type_info: &'static TypeInfo,
        target: &OverrideTarget,
    ) -> Result<SparseValue, PrefabCodecError> {
        let registration = self
            .registry
            .type_registry()
            .get(type_info.type_id())
            .ok_or_else(|| PrefabCodecError::UnregisteredOverrideValue {
                target: target.to_string(),
                type_path: type_info.type_path().to_owned(),
            })?;
        let value = deserialize_sparse(raw, registration, self.registry.type_registry())?;
        let value = SparseValue::for_type(value, type_info)?;
        validate_sparse_shape(type_info, value.value())?;
        Ok(value)
    }

    fn encode_override(
        &self,
        operation: &OverrideOperation,
    ) -> Result<RawOverrideOperation, PrefabCodecError> {
        let resolved = self
            .registry
            .resolve_prefab_type_path(&operation.target.component)
            .map_err(|error| match error {
                PrefabMigrationError::MissingPrefabTypeData { .. } => {
                    PrefabCodecError::NotPrefabComponent(operation.target.component.clone())
                }
                other => PrefabCodecError::Migration(other),
            })?;
        let target_type = type_at_path(
            resolved.source_registration.type_info(),
            operation.target.path.segments(),
            &operation.target.component,
        )?;
        let target = RawOverrideTarget {
            instance_chain: operation
                .target
                .instance_chain
                .iter()
                .map(|alias| alias.as_str().to_owned())
                .collect(),
            entity: operation.target.entity.as_str().to_owned(),
            component: operation.target.component.clone(),
            path: operation.target.path.to_string(),
        };
        let action = match &operation.action {
            OverrideAction::Set(value) => {
                validate_override_value(value, target_type, &operation.target)?;
                RawOverrideAction::Set(serialize_sparse(value, self.registry.type_registry())?)
            }
            OverrideAction::Clear => RawOverrideAction::Clear,
            OverrideAction::Insert { index, value } => {
                let element_type = collection_element(target_type, &operation.target)?;
                validate_override_value(value, element_type, &operation.target)?;
                RawOverrideAction::Insert {
                    index: *index,
                    value: serialize_sparse(value, self.registry.type_registry())?,
                }
            }
            OverrideAction::Remove { index } => {
                collection_element(target_type, &operation.target)?;
                RawOverrideAction::Remove { index: *index }
            }
            OverrideAction::Move { from, to } => RawOverrideAction::Move {
                from: {
                    collection_element(target_type, &operation.target)?;
                    *from
                },
                to: *to,
            },
        };
        Ok(RawOverrideOperation { target, action })
    }

    fn validate_canonical_document(
        &self,
        document: &PrefabDocument,
    ) -> Result<(), PrefabCodecError> {
        if document.version != PREFAB_DOCUMENT_VERSION {
            return Err(PrefabCodecError::UnsupportedDocumentVersion {
                actual: document.version,
                supported: PREFAB_DOCUMENT_VERSION,
            });
        }
        let mut used = BTreeSet::new();
        for (entity_alias, entity) in &document.entities {
            for (tag, value) in &entity.components {
                let resolved = self.registry.resolve_tag(tag)?;
                if resolved.prefab.tag != tag {
                    return Err(PrefabCodecError::NonCanonicalTag {
                        encoded: tag.clone(),
                        canonical: resolved.prefab.tag.to_owned(),
                    });
                }
                // Direct type-identity check. `SparseValue::for_type` only
                // compares the value's represented type-id against the expected
                // type-id (returning `ReflectedTypeMismatch`); the `to_dynamic()`
                // it required performed a full dynamic deep-copy per component
                // solely to feed that comparison — and for reflect-opaque leaf
                // types (e.g. `HashSet<EntityId>`) that copy would panic inside
                // `DynamicSet` hashing. `value.type_info()` already carries the
                // represented type-id, so compare it directly. Behavior-identical.
                if value.type_info().type_id() != resolved.source_registration.type_info().type_id()
                {
                    return Err(PrefabCodecError::Document(
                        PrefabDocumentError::ReflectedTypeMismatch {
                            expected: resolved
                                .source_registration
                                .type_info()
                                .type_path()
                                .to_owned(),
                            actual: value.type_path().to_owned(),
                        },
                    ));
                }
                validate_sparse_shape(resolved.source_registration.type_info(), value.value())?;
                let version = document
                    .type_versions
                    .get(tag)
                    .copied()
                    .ok_or_else(|| PrefabCodecError::MissingTypeVersion(tag.clone()))?;
                if version != resolved.prefab.source_version {
                    return Err(PrefabCodecError::NonCanonicalTypeVersion {
                        tag: tag.clone(),
                        actual: version,
                        current: resolved.prefab.source_version,
                    });
                }
                used.insert(tag.clone());
            }
            if entity_alias.as_str().is_empty() {
                return Err(PrefabCodecError::Document(
                    PrefabDocumentError::InvalidName {
                        kind: "entity alias",
                        value: String::new(),
                        reason: "value must be non-empty and trimmed",
                    },
                ));
            }
        }
        if let Some(unused) = document
            .type_versions
            .keys()
            .find(|tag| !used.contains(*tag))
        {
            return Err(PrefabCodecError::UnusedTypeVersion(unused.clone()));
        }
        Ok(())
    }
}

fn validate_override_value(
    value: &SparseValue,
    expected: &'static TypeInfo,
    target: &OverrideTarget,
) -> Result<(), PrefabCodecError> {
    if value.type_info().type_id() != expected.type_id() {
        return Err(PrefabCodecError::InvalidReflectedShape {
            type_path: target.to_string(),
            message: format!(
                "override value represents `{}`, expected `{}`",
                value.type_path(),
                expected.type_path()
            ),
        });
    }
    validate_sparse_shape(expected, value.value())
}

impl fmt::Display for OverrideTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.instance_chain.is_empty() {
            for instance in &self.instance_chain {
                write!(formatter, "{instance}/")?;
            }
        }
        write!(formatter, "{}:{}{}", self.entity, self.component, self.path)
    }
}

#[derive(Debug, Error)]
pub enum PrefabCodecError {
    #[error("failed to parse Prefab RON: {0}")]
    Parse(String),
    #[error("failed to serialize Prefab RON: {0}")]
    Serialize(String),
    #[error("unsupported Prefab document version {actual}; supported version is {supported}")]
    UnsupportedDocumentVersion { actual: u32, supported: u32 },
    #[error("component tag `{0}` has no type_versions entry")]
    MissingTypeVersion(String),
    #[error("type_versions contains unused tag `{0}`")]
    UnusedTypeVersion(String),
    #[error(
        "component `{tag}` occurs more than once under canonical identity on entity `{entity}`"
    )]
    DuplicateCanonicalComponent { entity: String, tag: String },
    #[error("source tag `{encoded}` is not canonical; write `{canonical}`")]
    NonCanonicalTag { encoded: String, canonical: String },
    #[error("type_versions declares {actual} for `{tag}`; canonical current version is {current}")]
    NonCanonicalTypeVersion {
        tag: String,
        actual: u32,
        current: u32,
    },
    #[error("failed to deserialize sparse reflected value for `{type_path}`: {message}")]
    ReflectDeserialize { type_path: String, message: String },
    #[error("failed to serialize sparse reflected value for `{type_path}`: {message}")]
    ReflectSerialize { type_path: String, message: String },
    #[error("reflected value for `{type_path}` has unknown field or incompatible shape: {message}")]
    InvalidReflectedShape { type_path: String, message: String },
    #[error("override component `{0}` is registered but is not Prefab-capable")]
    NotPrefabComponent(String),
    #[error("override target `{target}` requires unregistered value type `{type_path}`")]
    UnregisteredOverrideValue { target: String, type_path: String },
    #[error("override path `{path}` is invalid on `{type_path}`: {message}")]
    InvalidOverridePath {
        type_path: String,
        path: String,
        message: String,
    },
    #[error("override target `{target}` is not a list or array")]
    NotACollection { target: String },
    #[error(transparent)]
    Migration(#[from] PrefabMigrationError),
    #[error(transparent)]
    Document(#[from] PrefabDocumentError),
}

impl PrefabCodecError {
    fn parse(error: &ron::error::SpannedError) -> Self {
        Self::Parse(error.to_string())
    }

    fn serialize(error: &ron::Error) -> Self {
        Self::Serialize(error.to_string())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDocument {
    version: u32,
    type_versions: UniqueMap<String, u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    catalog_aliases: Vec<String>,
    entities: UniqueMap<String, RawEntity>,
    #[serde(default)]
    instances: UniqueMap<String, RawInstance>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntity {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    entity_id: Option<u64>,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    components: UniqueMap<String, Box<RawValue>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstance {
    source: String,
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    overrides: Vec<RawOverrideOperation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverrideOperation {
    target: RawOverrideTarget,
    action: RawOverrideAction,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOverrideTarget {
    #[serde(default)]
    instance_chain: Vec<String>,
    entity: String,
    component: String,
    path: String,
}

#[derive(Debug, Serialize, Deserialize)]
enum RawOverrideAction {
    Set(Box<RawValue>),
    Clear,
    Insert { index: usize, value: Box<RawValue> },
    Remove { index: usize },
    Move { from: usize, to: usize },
}

#[derive(Debug)]
pub(crate) struct UniqueMap<K, V>(pub(crate) BTreeMap<K, V>);

impl<K, V> Default for UniqueMap<K, V> {
    fn default() -> Self {
        Self(BTreeMap::new())
    }
}

impl<K, V> Serialize for UniqueMap<K, V>
where
    K: Serialize + Ord,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.serialize(serializer)
    }
}

impl<'de, K, V> Deserialize<'de> for UniqueMap<K, V>
where
    K: Deserialize<'de> + Ord + fmt::Display,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct UniqueMapVisitor<K, V>(std::marker::PhantomData<(K, V)>);

        impl<'de, K, V> Visitor<'de> for UniqueMapVisitor<K, V>
        where
            K: Deserialize<'de> + Ord + fmt::Display,
            V: Deserialize<'de>,
        {
            type Value = UniqueMap<K, V>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a map with unique keys")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = map.next_entry()? {
                    if values.insert(key, value).is_some() {
                        return Err(A::Error::custom("duplicate map key"));
                    }
                }
                Ok(UniqueMap(values))
            }
        }

        deserializer.deserialize_map(UniqueMapVisitor(std::marker::PhantomData))
    }
}

fn validate_declared_tags(
    registry: &PrefabRegistry<'_>,
    versions: &BTreeMap<String, u32>,
) -> Result<(), PrefabCodecError> {
    for (tag, version) in versions {
        let resolved = registry.resolve_tag(tag)?;
        if let Some(alias_version) = resolved.alias_version
            && *version != alias_version
        {
            return Err(PrefabMigrationError::AliasVersionMismatch {
                tag: tag.clone(),
                expected: alias_version,
                actual: *version,
            }
            .into());
        }
    }
    Ok(())
}

fn decode_target(raw: RawOverrideTarget) -> Result<OverrideTarget, PrefabCodecError> {
    let instance_chain = raw
        .instance_chain
        .into_iter()
        .map(InstanceAlias::new)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(OverrideTarget::new(
        instance_chain,
        EntityAlias::new(raw.entity)?,
        raw.component,
        ReflectedPath::parse(&raw.path)?,
    )?)
}

fn deserialize_sparse(
    raw: &RawValue,
    registration: &TypeRegistration,
    registry: &TypeRegistry,
) -> Result<Box<dyn PartialReflect>, PrefabCodecError> {
    let mut deserializer = ron::Deserializer::from_str(raw.get_ron()).map_err(|error| {
        PrefabCodecError::ReflectDeserialize {
            type_path: registration.type_info().type_path().to_owned(),
            message: error.to_string(),
        }
    })?;
    let mut processor = SparseDeserializeProcessor;
    let value = TypedReflectDeserializer::with_processor(registration, registry, &mut processor)
        .deserialize(&mut deserializer)
        .map_err(|error| PrefabCodecError::ReflectDeserialize {
            type_path: registration.type_info().type_path().to_owned(),
            message: error.to_string(),
        })?;
    deserializer
        .end()
        .map_err(|error| PrefabCodecError::ReflectDeserialize {
            type_path: registration.type_info().type_path().to_owned(),
            message: error.to_string(),
        })?;
    Ok(value)
}

fn serialize_sparse(
    value: &SparseValue,
    registry: &TypeRegistry,
) -> Result<Box<RawValue>, PrefabCodecError> {
    RawValue::from_rust(&TypedReflectSerializer::with_processor(
        value.value(),
        registry,
        &SparseSerializeProcessor,
    ))
    .map_err(|error| PrefabCodecError::ReflectSerialize {
        type_path: value.type_path().to_owned(),
        message: error.to_string(),
    })
}

struct SparseDeserializeProcessor;

impl ReflectDeserializerProcessor for SparseDeserializeProcessor {
    fn try_deserialize<'de, D>(
        &mut self,
        registration: &TypeRegistration,
        registry: &TypeRegistry,
        deserializer: D,
    ) -> Result<Result<Box<dyn PartialReflect>, D>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match registration.type_info() {
            // Enums always use their reflected variant vocabulary in Prefab
            // RON. Their serde adapter may intentionally speak another wire
            // (for example an integer ObjectStream discriminant), so it cannot
            // decide the authoring shape.
            TypeInfo::Enum(info) => {
                let mut value = deserializer.deserialize_enum(
                    info.type_path_table().ident().unwrap_or("SparseEnum"),
                    info.variant_names(),
                    SparseEnumVisitor {
                        info,
                        registry,
                        processor: self,
                    },
                )?;
                value.set_represented_type(Some(registration.type_info()));
                Ok(Ok(Box::new(value)))
            }
            // Serde-backed leaf values such as glam vectors and opaque asset
            // paths own their wire shape. Prefab registrations remain
            // structural even when they also expose serde adapters,
            // preserving sparse root fields.
            _ if registration.contains::<ReflectDeserialize>()
                && !registration.contains::<PrefabTypeData>() =>
            {
                Ok(Err(deserializer))
            }
            TypeInfo::Struct(info) => {
                let mut value = deserializer.deserialize_struct(
                    info.type_path_table().ident().unwrap_or("SparseStruct"),
                    info.field_names(),
                    SparseStructVisitor {
                        info,
                        registry,
                        processor: self,
                    },
                )?;
                value.set_represented_type(Some(registration.type_info()));
                Ok(Ok(Box::new(value)))
            }
            _ => Ok(Err(deserializer)),
        }
    }
}

struct SparseStructVisitor<'a> {
    info: &'static StructInfo,
    registry: &'a TypeRegistry,
    processor: &'a mut SparseDeserializeProcessor,
}

impl<'de> Visitor<'de> for SparseStructVisitor<'_> {
    type Value = DynamicStruct;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sparse reflected struct")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        visit_named_fields(&mut map, self.info, self.registry, self.processor)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut value = DynamicStruct::default();
        for (index, field) in self.info.iter().enumerate() {
            let registration = require_registration::<A::Error>(field.type_id(), self.registry)?;
            let field_value = sequence
                .next_element_seed(TypedReflectDeserializer::with_processor(
                    registration,
                    self.registry,
                    self.processor,
                ))?
                .ok_or_else(|| A::Error::invalid_length(index, &"all positional fields"))?;
            value.insert_boxed(field.name(), field_value);
        }
        Ok(value)
    }
}

trait NamedFields {
    fn type_label(&self) -> &str;
    fn field_named(&self, name: &str) -> Option<&bevy_reflect::NamedField>;
}

impl NamedFields for StructInfo {
    fn type_label(&self) -> &str {
        self.type_path()
    }

    fn field_named(&self, name: &str) -> Option<&bevy_reflect::NamedField> {
        self.field(name)
    }
}

impl NamedFields for StructVariantInfo {
    fn type_label(&self) -> &str {
        self.name()
    }

    fn field_named(&self, name: &str) -> Option<&bevy_reflect::NamedField> {
        self.field(name)
    }
}

fn visit_named_fields<'de, A, I>(
    map: &mut A,
    info: &'static I,
    registry: &TypeRegistry,
    processor: &mut SparseDeserializeProcessor,
) -> Result<DynamicStruct, A::Error>
where
    A: MapAccess<'de>,
    I: NamedFields,
{
    let mut value = DynamicStruct::default();
    while let Some(FieldName(name)) = map.next_key::<FieldName>()? {
        if value.field(&name).is_some() {
            return Err(A::Error::custom(format!("duplicate field `{name}`")));
        }
        let field = info.field_named(&name).ok_or_else(|| {
            A::Error::custom(format!("unknown field `{name}` on `{}`", info.type_label()))
        })?;
        let registration = require_registration::<A::Error>(field.type_id(), registry)?;
        let field_value = map.next_value_seed(TypedReflectDeserializer::with_processor(
            registration,
            registry,
            processor,
        ))?;
        value.insert_boxed(name, field_value);
    }
    Ok(value)
}

struct FieldName(String);

impl<'de> Deserialize<'de> for FieldName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct FieldNameVisitor;

        impl Visitor<'_> for FieldNameVisitor {
            type Value = FieldName;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a named reflected field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(FieldName(value.to_owned()))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(FieldName(value))
            }
        }

        deserializer.deserialize_identifier(FieldNameVisitor)
    }
}

struct SparseEnumVisitor<'a> {
    info: &'static EnumInfo,
    registry: &'a TypeRegistry,
    processor: &'a mut SparseDeserializeProcessor,
}

impl<'de> Visitor<'de> for SparseEnumVisitor<'_> {
    type Value = DynamicEnum;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sparse reflected enum")
    }

    fn visit_enum<A>(self, data: A) -> Result<Self::Value, A::Error>
    where
        A: EnumAccess<'de>,
    {
        let (variant, access) = data.variant_seed(VariantSeed { info: self.info })?;
        let dynamic = match variant {
            VariantInfo::Unit(_) => {
                access.unit_variant()?;
                DynamicVariant::Unit
            }
            VariantInfo::Struct(info) => DynamicVariant::Struct(access.struct_variant(
                info.field_names(),
                SparseStructVariantVisitor {
                    info,
                    registry: self.registry,
                    processor: self.processor,
                },
            )?),
            VariantInfo::Tuple(info) if info.field_len() == 1 => {
                let field = info.field_at(0).expect("single tuple variant field");
                let registration =
                    require_registration::<A::Error>(field.type_id(), self.registry)?;
                let field_value =
                    access.newtype_variant_seed(TypedReflectDeserializer::with_processor(
                        registration,
                        self.registry,
                        self.processor,
                    ))?;
                let mut tuple = DynamicTuple::default();
                tuple.insert_boxed(field_value);
                DynamicVariant::Tuple(tuple)
            }
            VariantInfo::Tuple(info) => DynamicVariant::Tuple(access.tuple_variant(
                info.field_len(),
                SparseTupleVariantVisitor {
                    info,
                    registry: self.registry,
                    processor: self.processor,
                },
            )?),
        };
        let index = self
            .info
            .index_of(variant.name())
            .expect("resolved enum variant belongs to EnumInfo");
        Ok(DynamicEnum::new_with_index(index, variant.name(), dynamic))
    }
}

struct VariantSeed {
    info: &'static EnumInfo,
}

impl<'de> DeserializeSeed<'de> for VariantSeed {
    type Value = &'static VariantInfo;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct VariantVisitor(&'static EnumInfo);

        impl Visitor<'_> for VariantVisitor {
            type Value = &'static VariantInfo;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a registered enum variant")
            }

            fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.0
                    .variant_at(value as usize)
                    .ok_or_else(|| E::custom(format!("unknown variant index {value}")))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.0
                    .variant(value)
                    .ok_or_else(|| E::custom(format!("unknown variant `{value}`")))
            }
        }

        deserializer.deserialize_identifier(VariantVisitor(self.info))
    }
}

struct SparseStructVariantVisitor<'a> {
    info: &'static StructVariantInfo,
    registry: &'a TypeRegistry,
    processor: &'a mut SparseDeserializeProcessor,
}

impl<'de> Visitor<'de> for SparseStructVariantVisitor<'_> {
    type Value = DynamicStruct;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a sparse reflected struct variant")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        visit_named_fields(&mut map, self.info, self.registry, self.processor)
    }
}

struct SparseTupleVariantVisitor<'a> {
    info: &'static bevy_reflect::enums::TupleVariantInfo,
    registry: &'a TypeRegistry,
    processor: &'a mut SparseDeserializeProcessor,
}

impl<'de> Visitor<'de> for SparseTupleVariantVisitor<'_> {
    type Value = DynamicTuple;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a reflected tuple variant")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut tuple = DynamicTuple::default();
        for (index, field) in self.info.iter().enumerate() {
            let registration = require_registration::<A::Error>(field.type_id(), self.registry)?;
            let value = sequence
                .next_element_seed(TypedReflectDeserializer::with_processor(
                    registration,
                    self.registry,
                    self.processor,
                ))?
                .ok_or_else(|| {
                    A::Error::custom(format!(
                        "expected {} tuple fields, found {index}",
                        self.info.field_len()
                    ))
                })?;
            tuple.insert_boxed(value);
        }
        Ok(tuple)
    }
}

fn require_registration<E: serde::de::Error>(
    type_id: std::any::TypeId,
    registry: &TypeRegistry,
) -> Result<&TypeRegistration, E> {
    registry
        .get(type_id)
        .ok_or_else(|| E::custom(format!("unregistered reflected field TypeId {type_id:?}")))
}

struct SparseSerializeProcessor;

impl ReflectSerializerProcessor for SparseSerializeProcessor {
    fn try_serialize<S>(
        &self,
        value: &dyn PartialReflect,
        registry: &TypeRegistry,
        serializer: S,
    ) -> Result<Result<S::Ok, S>, S::Error>
    where
        S: serde::Serializer,
    {
        // Mirror `SparseDeserializeProcessor`'s gate so encode/decode agree on
        // which values are RON-wire structural (Rust-ident keyed) versus
        // serde-adapter leaves. A registered `ReflectFromReflect` can
        // materialize a dynamic reflected value for `ReflectSerialize`, so
        // this policy must depend on the represented registration rather than
        // concrete-vs-dynamic storage.
        // Serde-backed leaves such as glam vectors and opaque asset paths own
        // their wire shape; Prefab registrations remain structural even when
        // they also expose a serde adapter, so a legacy
        // `#[serde(rename = "...")]` display name (used by the ObjectStream/
        // XML import path) never leaks into a RON struct key.
        let Some(type_info) = value.get_represented_type_info() else {
            return Ok(Err(serializer));
        };
        let Some(registration) = registry.get(type_info.type_id()) else {
            return Ok(Err(serializer));
        };
        match value.reflect_ref() {
            // Match deserialization: reflected enum variants are the canonical
            // Prefab vocabulary even when concrete serde uses a legacy integer
            // or tagged representation.
            ReflectRef::Enum(value) => Ok(Ok(SparseEnumSerializer {
                value,
                registry,
                processor: self,
            }
            .serialize(serializer)?)),
            _ if registration.contains::<ReflectSerialize>()
                && !registration.contains::<PrefabTypeData>() =>
            {
                let reflect_serialize = registration
                    .data::<ReflectSerialize>()
                    .expect("registration was checked for ReflectSerialize");
                if let Some(concrete) = value.try_as_reflect() {
                    return Ok(Ok(reflect_serialize.serialize(concrete, serializer)?));
                }
                let from_reflect = registration.data::<ReflectFromReflect>().ok_or_else(|| {
                    S::Error::custom(format_args!(
                        "dynamic serde-backed value `{}` has no ReflectFromReflect adapter",
                        type_info.type_path()
                    ))
                })?;
                let concrete = from_reflect.from_reflect(value).ok_or_else(|| {
                    S::Error::custom(format_args!(
                        "failed to materialize dynamic serde-backed value `{}`",
                        type_info.type_path()
                    ))
                })?;
                Ok(Ok(
                    reflect_serialize.serialize(concrete.as_ref(), serializer)?
                ))
            }
            ReflectRef::Struct(value) => Ok(Ok(SparseStructSerializer {
                value,
                registry,
                processor: self,
            }
            .serialize(serializer)?)),
            _ => Ok(Err(serializer)),
        }
    }
}

struct SparseStructSerializer<'a> {
    value: &'a dyn ReflectStruct,
    registry: &'a TypeRegistry,
    processor: &'a SparseSerializeProcessor,
}

impl Serialize for SparseStructSerializer<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let info = self
            .value
            .get_represented_type_info()
            .and_then(|info| info.as_struct().ok())
            .ok_or_else(|| S::Error::custom("dynamic sparse struct has no StructInfo"))?;
        let mut state = serializer.serialize_struct(
            info.type_path_table().ident().unwrap_or("SparseStruct"),
            self.value.field_len(),
        )?;
        for field_info in info.iter() {
            if let Some(field_value) = self.value.field(field_info.name()) {
                state.serialize_field(
                    field_info.name(),
                    &TypedReflectSerializer::with_processor(
                        field_value,
                        self.registry,
                        self.processor,
                    ),
                )?;
            }
        }
        state.end()
    }
}

struct SparseEnumSerializer<'a> {
    value: &'a dyn ReflectEnum,
    registry: &'a TypeRegistry,
    processor: &'a SparseSerializeProcessor,
}

impl Serialize for SparseEnumSerializer<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use bevy_reflect::enums::VariantType;
        use serde::ser::SerializeTupleVariant;

        let info = self
            .value
            .get_represented_type_info()
            .and_then(|info| info.as_enum().ok())
            .ok_or_else(|| S::Error::custom("dynamic sparse enum has no EnumInfo"))?;
        let variant = info
            .variant(self.value.variant_name())
            .ok_or_else(|| S::Error::custom("unknown sparse enum variant"))?;
        let enum_name = info.type_path_table().ident().unwrap_or("SparseEnum");
        let variant_name = variant.name();
        let index = u32::try_from(
            info.index_of(variant_name)
                .ok_or_else(|| S::Error::custom("unknown sparse enum variant"))?,
        )
        .map_err(|_| S::Error::custom("enum variant index exceeds u32"))?;
        match self.value.variant_type() {
            VariantType::Unit => serializer.serialize_unit_variant(enum_name, index, variant_name),
            VariantType::Tuple if self.value.field_len() == 1 => serializer
                .serialize_newtype_variant(
                    enum_name,
                    index,
                    variant_name,
                    &TypedReflectSerializer::with_processor(
                        self.value.field_at(0).expect("one tuple field"),
                        self.registry,
                        self.processor,
                    ),
                ),
            VariantType::Tuple => {
                let mut state = serializer.serialize_tuple_variant(
                    enum_name,
                    index,
                    variant_name,
                    self.value.field_len(),
                )?;
                for field in self.value.iter_fields() {
                    state.serialize_field(&TypedReflectSerializer::with_processor(
                        field.value(),
                        self.registry,
                        self.processor,
                    ))?;
                }
                state.end()
            }
            VariantType::Struct => {
                let VariantInfo::Struct(variant_info) = variant else {
                    return Err(S::Error::custom("enum variant shape mismatch"));
                };
                let mut state = serializer.serialize_struct_variant(
                    enum_name,
                    index,
                    variant_name,
                    self.value.field_len(),
                )?;
                for field_info in variant_info.iter() {
                    if let Some(field_value) = self.value.field(field_info.name()) {
                        state.serialize_field(
                            field_info.name(),
                            &TypedReflectSerializer::with_processor(
                                field_value,
                                self.registry,
                                self.processor,
                            ),
                        )?;
                    }
                }
                state.end()
            }
        }
    }
}

fn type_at_path(
    mut current: &'static TypeInfo,
    segments: &[String],
    root_type_path: &str,
) -> Result<&'static TypeInfo, PrefabCodecError> {
    for segment in segments {
        current = match current {
            TypeInfo::Struct(info) => info
                .field(segment)
                .and_then(bevy_reflect::NamedField::type_info)
                .ok_or_else(|| PrefabCodecError::InvalidOverridePath {
                    type_path: root_type_path.to_owned(),
                    path: ReflectedPath::new(segments.iter().cloned())
                        .map_or_else(|_| segments.join("."), |path| path.to_string()),
                    message: format!("unknown field `{segment}`"),
                })?,
            other => {
                return Err(PrefabCodecError::InvalidOverridePath {
                    type_path: root_type_path.to_owned(),
                    path: segments.join("."),
                    message: format!(
                        "cannot select named field `{segment}` through `{}`",
                        other.type_path()
                    ),
                });
            }
        };
    }
    Ok(current)
}

fn collection_element(
    type_info: &'static TypeInfo,
    target: &OverrideTarget,
) -> Result<&'static TypeInfo, PrefabCodecError> {
    match type_info {
        TypeInfo::List(info) => {
            info.item_info()
                .ok_or_else(|| PrefabCodecError::UnregisteredOverrideValue {
                    target: target.to_string(),
                    type_path: info.item_ty().path().to_owned(),
                })
        }
        TypeInfo::Array(info) => {
            info.item_info()
                .ok_or_else(|| PrefabCodecError::UnregisteredOverrideValue {
                    target: target.to_string(),
                    type_path: info.item_ty().path().to_owned(),
                })
        }
        _ => Err(PrefabCodecError::NotACollection {
            target: target.to_string(),
        }),
    }
}

/// Rejects a sparse value that carries no represented type information, or one
/// that represents a type other than `expected`.
fn validate_represented_type(
    expected: &'static TypeInfo,
    value: &dyn PartialReflect,
) -> Result<(), PrefabCodecError> {
    let represented = value.get_represented_type_info().ok_or_else(|| {
        PrefabCodecError::InvalidReflectedShape {
            type_path: expected.type_path().to_owned(),
            message: "value has no represented TypeInfo".to_owned(),
        }
    })?;
    if represented.type_id() != expected.type_id() {
        return Err(PrefabCodecError::InvalidReflectedShape {
            type_path: expected.type_path().to_owned(),
            message: format!("value represents `{}`", represented.type_path()),
        });
    }
    Ok(())
}

/// Validates every retained named field of a sparse struct against `info`.
fn validate_sparse_struct(
    expected: &'static TypeInfo,
    info: &StructInfo,
    value: &dyn ReflectStruct,
) -> Result<(), PrefabCodecError> {
    for (name, field_value) in value.iter_fields() {
        let field = info
            .field(name)
            .ok_or_else(|| PrefabCodecError::InvalidReflectedShape {
                type_path: expected.type_path().to_owned(),
                message: format!("unknown field `{name}` after migration"),
            })?;
        validate_sparse_shape(
            required_field_info(expected, field.type_info())?,
            field_value,
        )?;
    }
    Ok(())
}

/// Validates that a sparse enum names a known variant of `info`, that the
/// variant kinds agree, and that every retained variant field fits.
fn validate_sparse_enum(
    expected: &'static TypeInfo,
    info: &EnumInfo,
    value: &dyn ReflectEnum,
) -> Result<(), PrefabCodecError> {
    let variant = info.variant(value.variant_name()).ok_or_else(|| {
        PrefabCodecError::InvalidReflectedShape {
            type_path: expected.type_path().to_owned(),
            message: format!("unknown variant `{}` after migration", value.variant_name()),
        }
    })?;
    if variant.variant_type() != value.variant_type() {
        return Err(PrefabCodecError::InvalidReflectedShape {
            type_path: expected.type_path().to_owned(),
            message: format!(
                "variant `{}` has incompatible reflected shape",
                value.variant_name()
            ),
        });
    }
    match variant {
        VariantInfo::Struct(variant) => {
            for field in value.iter_fields() {
                let name = field.name().unwrap_or_default();
                let field_info =
                    variant
                        .field(name)
                        .ok_or_else(|| PrefabCodecError::InvalidReflectedShape {
                            type_path: expected.type_path().to_owned(),
                            message: format!("unknown variant field `{name}` after migration"),
                        })?;
                validate_sparse_shape(
                    required_field_info(expected, field_info.type_info())?,
                    field.value(),
                )?;
            }
        }
        VariantInfo::Tuple(variant) => {
            if value.field_len() != variant.field_len() {
                return Err(PrefabCodecError::InvalidReflectedShape {
                    type_path: expected.type_path().to_owned(),
                    message: "tuple variant field count mismatch".to_owned(),
                });
            }
            for (index, field) in value.iter_fields().enumerate() {
                let field_info = variant.field_at(index).expect("validated field count");
                validate_sparse_shape(
                    required_field_info(expected, field_info.type_info())?,
                    field.value(),
                )?;
            }
        }
        VariantInfo::Unit(_) if value.field_len() == 0 => {}
        VariantInfo::Unit(_) => {
            return Err(PrefabCodecError::InvalidReflectedShape {
                type_path: expected.type_path().to_owned(),
                message: "unit variant carries fields".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_sparse_shape(
    expected: &'static TypeInfo,
    value: &dyn PartialReflect,
) -> Result<(), PrefabCodecError> {
    validate_represented_type(expected, value)?;

    match (expected, value.reflect_ref()) {
        (TypeInfo::Struct(info), ReflectRef::Struct(value)) => {
            validate_sparse_struct(expected, info, value)?;
        }
        (TypeInfo::Enum(info), ReflectRef::Enum(value)) => {
            validate_sparse_enum(expected, info, value)?;
        }
        (TypeInfo::List(info), ReflectRef::List(value)) => {
            let item_info = required_field_info(expected, info.item_info())?;
            for field in value.iter() {
                validate_sparse_shape(item_info, field)?;
            }
        }
        (TypeInfo::Array(info), ReflectRef::Array(value)) => {
            let item_info = required_field_info(expected, info.item_info())?;
            for field in value.iter() {
                validate_sparse_shape(item_info, field)?;
            }
        }
        (TypeInfo::TupleStruct(info), ReflectRef::TupleStruct(value)) => {
            if value.field_len() != info.field_len() {
                return Err(PrefabCodecError::InvalidReflectedShape {
                    type_path: expected.type_path().to_owned(),
                    message: "tuple struct field count mismatch".to_owned(),
                });
            }
            for (index, field) in value.iter_fields().enumerate() {
                let field_info = info.field_at(index).expect("validated length");
                validate_sparse_shape(
                    required_field_info(expected, field_info.type_info())?,
                    field,
                )?;
            }
        }
        (TypeInfo::Tuple(info), ReflectRef::Tuple(value)) => {
            if value.field_len() != info.field_len() {
                return Err(PrefabCodecError::InvalidReflectedShape {
                    type_path: expected.type_path().to_owned(),
                    message: "tuple field count mismatch".to_owned(),
                });
            }
            for (index, field) in value.iter_fields().enumerate() {
                let field_info = info.field_at(index).expect("validated length");
                validate_sparse_shape(
                    required_field_info(expected, field_info.type_info())?,
                    field,
                )?;
            }
        }
        (TypeInfo::Map(_), ReflectRef::Map(_))
        | (TypeInfo::Set(_), ReflectRef::Set(_))
        | (TypeInfo::Opaque(_), ReflectRef::Opaque(_)) => {}
        _ => {
            return Err(PrefabCodecError::InvalidReflectedShape {
                type_path: expected.type_path().to_owned(),
                message: "reflection kind mismatch".to_owned(),
            });
        }
    }
    Ok(())
}

fn required_field_info(
    owner: &'static TypeInfo,
    field: Option<&'static TypeInfo>,
) -> Result<&'static TypeInfo, PrefabCodecError> {
    field.ok_or_else(|| PrefabCodecError::InvalidReflectedShape {
        type_path: owner.type_path().to_owned(),
        message: "field has no static TypeInfo".to_owned(),
    })
}
