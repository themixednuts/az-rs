//! AZSCENE version 1 binary envelope.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Cursor, Read, Write},
};

use az_asset::AssetId as AzAssetId;
use az_core::EntityId;
use bevy::{
    asset::{
        AssetId, AssetLoader, AsyncReadExt, LoadContext, LoadFromPath, UntypedHandle, io::Reader,
    },
    ecs::{
        entity::Entity,
        reflect::AppTypeRegistry,
        world::{FromWorld, World},
    },
    reflect::{
        PartialReflect, ReflectFromReflect, TypePath, TypeRegistry, TypeRegistryArc,
        serde::TypedReflectDeserializer,
    },
    world_serialization::{DynamicEntity, DynamicWorld},
};
use serde::{Deserialize, de::DeserializeSeed};
use thiserror::Error;

use crate::{
    AzSceneAsset, AzSceneComponentTarget, AzSceneEntityMetadata, AzSceneMetadata,
    AzSceneSourceScopeMetadata, LocalEntityId, LocalEntityScopeId,
    asset_handles::{SceneDeserializeProcessor, SceneSerializeProcessor, TrackingLoadFromPath},
    reflected_value_wire::ReflectedValueWire,
};

pub const AZSCENE_MAGIC: &[u8; 8] = b"AZSCENE\0";
pub const AZSCENE_FORMAT_VERSION: u32 = 1;
pub const AZSCENE_PRODUCT_EXTENSION: &str = "scn.bin";

const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;
const MAX_TYPE_COUNT: u32 = 64 * 1024;
const MAX_DEPENDENCY_COUNT: u32 = 64 * 1024;
const MAX_RESOURCE_COUNT: u32 = 64 * 1024;
const MAX_ENTITY_COUNT: u32 = 1024 * 1024;
const MAX_SOURCE_SCOPE_COUNT: u32 = 1024 * 1024;
const MAX_COMPONENT_COUNT: u32 = 64 * 1024;
const MAX_TYPE_PATH_BYTES: u32 = 16 * 1024;
const MAX_ALIAS_BYTES: u32 = 16 * 1024;
const MAX_DEPENDENCY_PATH_BYTES: u32 = 64 * 1024;
const MAX_PAYLOAD_BYTES: u32 = 64 * 1024 * 1024;
const NO_PARENT: u32 = u32::MAX;

/// Canonical bytes plus path-handle and canonical product dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedAzScene {
    pub bytes: Vec<u8>,
    /// Bevy path-handle dependencies embedded in the versioned product.
    pub dependencies: Vec<String>,
    /// Canonical AZ product identities persisted by the asset processor.
    pub asset_dependencies: Vec<AzAssetId>,
}

struct EncodedValue {
    type_index: u32,
    payload: Vec<u8>,
}

struct EncodedEntity {
    local_id: u32,
    source_alias: String,
    source_scope: LocalEntityScopeId,
    source_entity_id: Option<EntityId>,
    parent: Option<u32>,
    component_targets: Vec<AzSceneComponentTarget>,
    components: Vec<EncodedValue>,
}

/// Encodes one product completely in memory.
///
/// Entity records retain `DynamicWorld` order. The processing path establishes that
/// order by allocating authored aliases lexically, followed by generated
/// flattened aliases lexically. Components, resources, map/set entries, type
/// paths, and dependency paths are sorted by the codec.
///
/// # Errors
///
/// Returns an error if the asset metadata is invalid, a component or resource
/// type is missing its represented type or is not registered in `registry`, or
/// a table, chunk, or the whole file exceeds the format's count and size
/// limits.
pub fn encode_scene_asset(
    asset: &AzSceneAsset,
    registry: &TypeRegistry,
) -> Result<EncodedAzScene, AzSceneCodecError> {
    validate_metadata(asset)?;
    let type_table = type_table(&asset.dynamic_world, registry)?;
    let entity_ids = asset
        .dynamic_world
        .entities
        .iter()
        .enumerate()
        .map(|(index, entity)| {
            let index = u32::try_from(index).map_err(|_| AzSceneCodecError::LengthOverflow {
                kind: "entities",
                len: index,
            })?;
            Ok((entity.entity, index))
        })
        .collect::<Result<BTreeMap<_, _>, AzSceneCodecError>>()?;
    let processor = SceneSerializeProcessor::new(&entity_ids);

    let resources = sorted_values(&asset.dynamic_world.resources)?
        .into_iter()
        .map(|(type_path, value)| encode_value(type_path, value, &type_table, registry, &processor))
        .collect::<Result<Vec<_>, _>>()?;
    let mut entities = Vec::with_capacity(asset.dynamic_world.entities.len());
    for (index, (entity, metadata)) in asset
        .dynamic_world
        .entities
        .iter()
        .zip(&asset.metadata.entities)
        .enumerate()
    {
        let local_id = u32::try_from(index).map_err(|_| AzSceneCodecError::LengthOverflow {
            kind: "entities",
            len: index,
        })?;
        let components = sorted_values(&entity.components)?
            .into_iter()
            .map(|(type_path, value)| {
                encode_value(type_path, value, &type_table, registry, &processor)
            })
            .collect::<Result<Vec<_>, _>>()?;
        entities.push(EncodedEntity {
            local_id,
            source_alias: metadata.source_alias.clone(),
            source_scope: metadata.source_scope,
            source_entity_id: metadata.source_entity_id,
            parent: metadata.parent.map(LocalEntityId::value),
            component_targets: metadata.component_targets.clone(),
            components,
        });
    }
    let dependencies = processor.dependencies();
    let asset_dependencies = processor.asset_dependencies();

    let bytes = write_scene_bytes(
        &type_table,
        &dependencies,
        &asset.metadata.source_scopes,
        &resources,
        &entities,
    )?;

    Ok(EncodedAzScene {
        bytes,
        dependencies,
        asset_dependencies,
    })
}

/// Serializes the already-encoded scene model into canonical product bytes.
fn write_scene_bytes(
    type_table: &[String],
    dependencies: &[String],
    source_scopes: &[AzSceneSourceScopeMetadata],
    resources: &[EncodedValue],
    entities: &[EncodedEntity],
) -> Result<Vec<u8>, AzSceneCodecError> {
    let mut bytes = Vec::new();
    bytes.write_all(AZSCENE_MAGIC)?;
    write_u32(&mut bytes, AZSCENE_FORMAT_VERSION)?;
    write_count(&mut bytes, "types", type_table.len(), MAX_TYPE_COUNT)?;
    write_count(
        &mut bytes,
        "dependencies",
        dependencies.len(),
        MAX_DEPENDENCY_COUNT,
    )?;
    write_count(&mut bytes, "resources", resources.len(), MAX_RESOURCE_COUNT)?;
    write_count(
        &mut bytes,
        "source scopes",
        source_scopes.len(),
        MAX_SOURCE_SCOPE_COUNT,
    )?;
    write_count(&mut bytes, "entities", entities.len(), MAX_ENTITY_COUNT)?;
    for type_path in type_table {
        write_bounded_string(&mut bytes, "type path", type_path, MAX_TYPE_PATH_BYTES)?;
    }
    for dependency in dependencies {
        write_bounded_string(
            &mut bytes,
            "dependency path",
            dependency,
            MAX_DEPENDENCY_PATH_BYTES,
        )?;
    }
    for (index, scope) in source_scopes.iter().enumerate() {
        write_u32(
            &mut bytes,
            u32::try_from(index).map_err(|_| AzSceneCodecError::LengthOverflow {
                kind: "source scopes",
                len: index,
            })?,
        )?;
        write_u32(
            &mut bytes,
            scope.parent.map_or(NO_PARENT, LocalEntityScopeId::value),
        )?;
    }
    for resource in resources {
        write_value(&mut bytes, resource)?;
    }
    for entity in entities {
        write_u32(&mut bytes, entity.local_id)?;
        write_bounded_string(
            &mut bytes,
            "source alias",
            &entity.source_alias,
            MAX_ALIAS_BYTES,
        )?;
        write_source_entity_id(&mut bytes, entity.source_entity_id)?;
        write_u32(&mut bytes, entity.source_scope.value())?;
        write_u32(&mut bytes, entity.parent.unwrap_or(NO_PARENT))?;
        write_count(
            &mut bytes,
            "component targets",
            entity.component_targets.len(),
            MAX_COMPONENT_COUNT,
        )?;
        for target in &entity.component_targets {
            bytes.write_all(target.native_type_id.as_bytes())?;
            write_u64(&mut bytes, target.component_id.value())?;
        }
        write_count(
            &mut bytes,
            "components",
            entity.components.len(),
            MAX_COMPONENT_COUNT,
        )?;
        for component in &entity.components {
            write_value(&mut bytes, component)?;
        }
    }
    if bytes.len() > MAX_FILE_BYTES {
        return Err(AzSceneCodecError::FileTooLarge {
            len: bytes.len(),
            max: MAX_FILE_BYTES,
        });
    }

    Ok(bytes)
}

/// Writes canonical version 1 bytes and returns the dependency ledger.
///
/// # Errors
///
/// Returns an error if the asset fails to encode, or `writer` rejects the
/// bytes.
pub fn write_scene_asset(
    asset: &AzSceneAsset,
    registry: &TypeRegistry,
    mut writer: impl Write,
) -> Result<Vec<String>, AzSceneCodecError> {
    let encoded = encode_scene_asset(asset, registry)?;
    writer.write_all(&encoded.bytes)?;
    Ok(encoded.dependencies)
}

/// Reads a dependency-free product. Products containing path handles must use
/// [`read_scene_asset_with_loader`] so every path reaches a real load context.
///
/// # Errors
///
/// Returns an error if the envelope is malformed or its version unsupported, a
/// decoded type is absent from `registry` or the type table, canonical
/// ordering is violated, or the product carries a path handle — which requires
/// [`read_scene_asset_with_loader`].
pub fn read_scene_asset_from_reader(
    reader: impl Read,
    registry: &TypeRegistry,
) -> Result<AzSceneAsset, AzSceneCodecError> {
    let mut reject = RejectPathLoads::default();
    decode_scene_asset(reader, registry, &mut reject, false)
}

/// Reads a product while resolving every reflected path handle through the
/// supplied Bevy loader and verifies the observed paths exactly match the
/// product dependency table.
///
/// # Errors
///
/// Returns an error if the envelope is malformed or its version unsupported, a
/// decoded type is absent from `registry` or the type table, canonical
/// ordering is violated, or the observed path loads do not exactly match the
/// product dependency table.
pub fn read_scene_asset_with_loader(
    reader: impl Read,
    registry: &TypeRegistry,
    load_from_path: &mut dyn LoadFromPath,
) -> Result<AzSceneAsset, AzSceneCodecError> {
    decode_scene_asset(reader, registry, load_from_path, true)
}

/// Reads and validates only the product metadata required by headless systems.
///
/// Reflected resource and component payloads remain opaque. This keeps servers
/// independent of the Bevy type registry while preserving the exact same
/// envelope, canonical-order, hierarchy, and component-target checks as a full
/// scene load.
///
/// # Errors
///
/// Returns an error if the envelope is malformed or its version unsupported,
/// the dependency or type table is not canonical, or the hierarchy and
/// component-target checks fail.
pub fn read_scene_metadata_from_reader(
    mut reader: impl Read,
) -> Result<AzSceneMetadata, AzSceneCodecError> {
    let header = read_header(&mut reader)?;
    validate_dependency_paths(&header.dependencies)?;

    let mut used_types = BTreeSet::new();
    for _ in 0..header.resource_count {
        skip_value(&mut reader, &header.type_table, &mut used_types)?;
    }

    let entity_count_u32 =
        u32::try_from(header.entity_count).map_err(|_| AzSceneCodecError::LengthOverflow {
            kind: "entities",
            len: header.entity_count,
        })?;
    let mut metadata = Vec::with_capacity(header.entity_count);
    let mut aliases = BTreeSet::new();
    for expected in 0..entity_count_u32 {
        let local_id = read_u32(&mut reader)?;
        if local_id != expected {
            return Err(AzSceneCodecError::NonCanonicalEntityOrder {
                expected,
                found: local_id,
            });
        }
        let source_alias = read_bounded_string(&mut reader, "source alias", MAX_ALIAS_BYTES)?;
        if source_alias.is_empty() || !aliases.insert(source_alias.clone()) {
            return Err(AzSceneCodecError::InvalidSourceAlias { source_alias });
        }
        let source_entity_id = read_source_entity_id(&mut reader, local_id)?;
        let source_scope = LocalEntityScopeId::new(read_u32(&mut reader)?);
        let parent = read_parent(&mut reader, local_id, entity_count_u32)?;
        let component_targets = read_component_targets(&mut reader, local_id)?;
        let component_count = read_count(&mut reader, "components", MAX_COMPONENT_COUNT)?;
        let mut previous_type = None;
        for _ in 0..component_count {
            let type_index = skip_value(&mut reader, &header.type_table, &mut used_types)?;
            if previous_type.is_some_and(|previous| previous >= type_index) {
                return Err(AzSceneCodecError::NonCanonicalComponentOrder { entity: local_id });
            }
            previous_type = Some(type_index);
        }
        metadata.push(AzSceneEntityMetadata {
            source_alias,
            source_scope,
            source_entity_id,
            parent,
            component_targets,
        });
    }
    read_trailer(&mut reader)?;
    validate_used_types(&header.type_table, &used_types)?;
    let metadata = AzSceneMetadata {
        source_scopes: header.source_scopes,
        entities: metadata,
    };
    validate_metadata_entries(&metadata)?;
    Ok(metadata)
}

struct SceneHeader {
    type_table: Vec<String>,
    dependencies: Vec<String>,
    source_scopes: Vec<AzSceneSourceScopeMetadata>,
    resource_count: usize,
    entity_count: usize,
}

fn read_header(reader: &mut impl Read) -> Result<SceneHeader, AzSceneCodecError> {
    let mut magic = [0_u8; 8];
    reader.read_exact(&mut magic)?;
    if &magic != AZSCENE_MAGIC {
        return Err(AzSceneCodecError::BadMagic { found: magic });
    }
    let version = read_u32(reader)?;
    if version != AZSCENE_FORMAT_VERSION {
        return Err(AzSceneCodecError::UnsupportedVersion {
            version,
            expected: AZSCENE_FORMAT_VERSION,
        });
    }

    let type_count = read_count(reader, "types", MAX_TYPE_COUNT)?;
    let dependency_count = read_count(reader, "dependencies", MAX_DEPENDENCY_COUNT)?;
    let resource_count = read_count(reader, "resources", MAX_RESOURCE_COUNT)?;
    let source_scope_count = read_count(reader, "source scopes", MAX_SOURCE_SCOPE_COUNT)?;
    let entity_count = read_count(reader, "entities", MAX_ENTITY_COUNT)?;
    let type_table = read_sorted_table(reader, "type path", type_count, MAX_TYPE_PATH_BYTES)?;
    let dependencies = read_sorted_table(
        reader,
        "dependency path",
        dependency_count,
        MAX_DEPENDENCY_PATH_BYTES,
    )?;
    let source_scopes = read_source_scopes(reader, source_scope_count)?;
    Ok(SceneHeader {
        type_table,
        dependencies,
        source_scopes,
        resource_count,
        entity_count,
    })
}

/// Rejects a decoded type table holding an unregistered or non-canonical path.
fn validate_type_table(
    type_table: &[String],
    registry: &TypeRegistry,
) -> Result<(), AzSceneCodecError> {
    for type_path in type_table {
        let registration = registry.get_with_type_path(type_path).ok_or_else(|| {
            AzSceneCodecError::UnregisteredType {
                type_path: type_path.clone(),
            }
        })?;
        if registration.type_info().type_path() != type_path {
            return Err(AzSceneCodecError::NonCanonicalTypePath {
                encoded: type_path.clone(),
                canonical: registration.type_info().type_path().to_owned(),
            });
        }
    }
    Ok(())
}

fn decode_scene_asset(
    mut reader: impl Read,
    registry: &TypeRegistry,
    load_from_path: &mut dyn LoadFromPath,
    allow_dependencies: bool,
) -> Result<AzSceneAsset, AzSceneCodecError> {
    let header = read_header(&mut reader)?;
    validate_type_table(&header.type_table, registry)?;
    if !allow_dependencies && !header.dependencies.is_empty() {
        return Err(AzSceneCodecError::AssetLoaderRequired);
    }
    validate_dependency_paths(&header.dependencies)?;

    let entity_count_u32 =
        u32::try_from(header.entity_count).map_err(|_| AzSceneCodecError::LengthOverflow {
            kind: "entities",
            len: header.entity_count,
        })?;
    let mut tracking = TrackingLoadFromPath::new(load_from_path);
    let mut processor = SceneDeserializeProcessor::new(entity_count_u32, &mut tracking);
    let mut used_types = BTreeSet::new();
    let mut resources = Vec::with_capacity(header.resource_count);
    for _ in 0..header.resource_count {
        resources.push(read_value(
            &mut reader,
            &header.type_table,
            registry,
            &mut processor,
            &mut used_types,
        )?);
    }

    let mut entities = Vec::with_capacity(header.entity_count);
    let mut metadata = Vec::with_capacity(header.entity_count);
    let mut aliases = BTreeSet::new();
    for expected in 0..entity_count_u32 {
        let local_id = read_u32(&mut reader)?;
        if local_id != expected {
            return Err(AzSceneCodecError::NonCanonicalEntityOrder {
                expected,
                found: local_id,
            });
        }
        let source_alias = read_bounded_string(&mut reader, "source alias", MAX_ALIAS_BYTES)?;
        if source_alias.is_empty() || !aliases.insert(source_alias.clone()) {
            return Err(AzSceneCodecError::InvalidSourceAlias { source_alias });
        }
        let source_entity_id = read_source_entity_id(&mut reader, local_id)?;
        let source_scope = LocalEntityScopeId::new(read_u32(&mut reader)?);
        let parent = read_parent(&mut reader, local_id, entity_count_u32)?;
        let component_targets = read_component_targets(&mut reader, local_id)?;
        let component_count = read_count(&mut reader, "components", MAX_COMPONENT_COUNT)?;
        let mut components = Vec::with_capacity(component_count);
        let mut previous_type = None;
        for _ in 0..component_count {
            let (type_index, value) = read_value_with_index(
                &mut reader,
                &header.type_table,
                registry,
                &mut processor,
                &mut used_types,
            )?;
            if previous_type.is_some_and(|previous| previous >= type_index) {
                return Err(AzSceneCodecError::NonCanonicalComponentOrder { entity: local_id });
            }
            previous_type = Some(type_index);
            components.push(value);
        }
        let entity = Entity::from_raw_u32(local_id)
            .ok_or(AzSceneCodecError::InvalidLocalEntity { local_id })?;
        entities.push(DynamicEntity { entity, components });
        metadata.push(AzSceneEntityMetadata {
            source_alias,
            source_scope,
            source_entity_id,
            parent,
            component_targets,
        });
    }
    read_trailer(&mut reader)?;

    // `processor` holds the `&mut tracking` borrow; its last use is above, so the
    // borrow has already ended here and the ledger can be read back.
    let observed_dependencies = tracking.dependencies();
    if observed_dependencies != header.dependencies {
        return Err(AzSceneCodecError::DependencyLedgerMismatch {
            declared: header.dependencies,
            observed: observed_dependencies,
        });
    }
    validate_used_types(&header.type_table, &used_types)?;
    let metadata = AzSceneMetadata {
        source_scopes: header.source_scopes,
        entities: metadata,
    };
    validate_metadata_entries(&metadata)?;

    Ok(AzSceneAsset {
        dynamic_world: DynamicWorld {
            resources,
            entities,
        },
        dependencies: observed_dependencies,
        metadata,
    })
}

fn encode_value(
    type_path: &str,
    value: &dyn PartialReflect,
    type_table: &[String],
    registry: &TypeRegistry,
    processor: &SceneSerializeProcessor<'_>,
) -> Result<EncodedValue, AzSceneCodecError> {
    let index = type_table
        .binary_search_by(|entry| entry.as_str().cmp(type_path))
        .map_err(|_| AzSceneCodecError::TypeNotInTable {
            type_path: type_path.to_owned(),
        })?;
    let type_index = u32::try_from(index).map_err(|_| AzSceneCodecError::LengthOverflow {
        kind: "type index",
        len: index,
    })?;
    let reflected_value = serde_value::to_value(
        bevy::reflect::serde::TypedReflectSerializer::with_processor(value, registry, processor),
    )
    .map_err(|source| AzSceneCodecError::ReflectedValueSerialization {
        type_path: type_path.to_owned(),
        source,
    })?;
    let payload =
        postcard::to_allocvec(&ReflectedValueWire::from(reflected_value)).map_err(|source| {
            AzSceneCodecError::Postcard {
                type_path: type_path.to_owned(),
                source,
            }
        })?;
    if payload.len() > usize::try_from(MAX_PAYLOAD_BYTES).unwrap_or(usize::MAX) {
        return Err(AzSceneCodecError::ChunkTooLarge {
            kind: "payload",
            len: u32::try_from(payload.len()).unwrap_or(u32::MAX),
            max: MAX_PAYLOAD_BYTES,
        });
    }
    Ok(EncodedValue {
        type_index,
        payload,
    })
}

fn read_value(
    reader: &mut impl Read,
    type_table: &[String],
    registry: &TypeRegistry,
    processor: &mut SceneDeserializeProcessor<'_>,
    used_types: &mut BTreeSet<u32>,
) -> Result<Box<dyn PartialReflect>, AzSceneCodecError> {
    read_value_with_index(reader, type_table, registry, processor, used_types)
        .map(|(_, value)| value)
}

fn read_value_with_index(
    reader: &mut impl Read,
    type_table: &[String],
    registry: &TypeRegistry,
    processor: &mut SceneDeserializeProcessor<'_>,
    used_types: &mut BTreeSet<u32>,
) -> Result<(u32, Box<dyn PartialReflect>), AzSceneCodecError> {
    let type_index = read_u32(reader)?;
    let index = usize::try_from(type_index).map_err(|_| AzSceneCodecError::InvalidTypeIndex {
        index: type_index,
        type_count: type_table.len(),
    })?;
    let type_path = type_table
        .get(index)
        .ok_or(AzSceneCodecError::InvalidTypeIndex {
            index: type_index,
            type_count: type_table.len(),
        })?;
    used_types.insert(type_index);
    let payload = read_bounded_bytes(reader, "payload", MAX_PAYLOAD_BYTES)?;
    let registration = registry.get_with_type_path(type_path).ok_or_else(|| {
        AzSceneCodecError::UnregisteredType {
            type_path: type_path.clone(),
        }
    })?;
    let mut deserializer = postcard::Deserializer::from_bytes(&payload);
    let reflected_value = ReflectedValueWire::deserialize(&mut deserializer).map_err(|source| {
        AzSceneCodecError::Postcard {
            type_path: type_path.clone(),
            source,
        }
    })?;
    let trailing = deserializer
        .finalize()
        .map_err(|source| AzSceneCodecError::Postcard {
            type_path: type_path.clone(),
            source,
        })?;
    if !trailing.is_empty() {
        return Err(AzSceneCodecError::TrailingPayloadBytes {
            type_path: type_path.clone(),
            trailing: trailing.len(),
        });
    }
    let reflected_value = serde_value::Value::try_from(reflected_value).map_err(|source| {
        AzSceneCodecError::InvalidReflectedValue {
            type_path: type_path.clone(),
            reason: source.to_string(),
        }
    })?;
    let value = TypedReflectDeserializer::with_processor(registration, registry, processor)
        .deserialize(reflected_value)
        .map_err(|source| AzSceneCodecError::ReflectedValueDeserialization {
            type_path: type_path.clone(),
            source,
        })?;
    let value = registration
        .data::<ReflectFromReflect>()
        .and_then(|reflect| reflect.from_reflect(value.as_partial_reflect()))
        .map(PartialReflect::into_partial_reflect)
        .unwrap_or(value);
    Ok((type_index, value))
}

fn skip_value(
    reader: &mut impl Read,
    type_table: &[String],
    used_types: &mut BTreeSet<u32>,
) -> Result<u32, AzSceneCodecError> {
    let type_index = read_u32(reader)?;
    let index = usize::try_from(type_index).map_err(|_| AzSceneCodecError::InvalidTypeIndex {
        index: type_index,
        type_count: type_table.len(),
    })?;
    if type_table.get(index).is_none() {
        return Err(AzSceneCodecError::InvalidTypeIndex {
            index: type_index,
            type_count: type_table.len(),
        });
    }
    used_types.insert(type_index);
    discard_bounded_bytes(reader, "payload", MAX_PAYLOAD_BYTES)?;
    Ok(type_index)
}

fn write_source_entity_id(
    writer: &mut impl Write,
    source_entity_id: Option<EntityId>,
) -> Result<(), AzSceneCodecError> {
    match source_entity_id {
        None => write_u8(writer, 0)?,
        Some(source_entity_id) => {
            write_u8(writer, 1)?;
            write_u64(writer, source_entity_id.value())?;
        }
    }
    Ok(())
}

fn read_source_entity_id(
    reader: &mut impl Read,
    entity: u32,
) -> Result<Option<EntityId>, AzSceneCodecError> {
    match read_u8(reader)? {
        0 => Ok(None),
        1 => {
            let source_entity_id = EntityId::new(read_u64(reader)?);
            if source_entity_id.is_invalid() {
                return Err(AzSceneCodecError::InvalidSourceEntityId { entity });
            }
            Ok(Some(source_entity_id))
        }
        tag => Err(AzSceneCodecError::InvalidOptionTag {
            kind: "source entity id",
            tag,
        }),
    }
}

fn read_parent(
    reader: &mut impl Read,
    local_id: u32,
    entity_count: u32,
) -> Result<Option<LocalEntityId>, AzSceneCodecError> {
    match read_u32(reader)? {
        NO_PARENT => Ok(None),
        parent if parent < entity_count && parent != local_id => {
            Ok(Some(LocalEntityId::new(parent)))
        }
        parent => Err(AzSceneCodecError::InvalidParent {
            entity: local_id,
            parent,
            entity_count,
        }),
    }
}

fn read_component_targets(
    reader: &mut impl Read,
    entity: u32,
) -> Result<Vec<AzSceneComponentTarget>, AzSceneCodecError> {
    let target_count = read_count(reader, "component targets", MAX_COMPONENT_COUNT)?;
    let mut component_targets = Vec::with_capacity(target_count);
    let mut previous_target = None;
    let mut local_ids = BTreeSet::new();
    for _ in 0..target_count {
        let mut type_bytes = [0_u8; 16];
        reader.read_exact(&mut type_bytes)?;
        let native_type_id = uuid::Uuid::from_bytes(type_bytes);
        let component_id = az_core::component::ComponentId::new(read_u64(reader)?);
        if !component_id.is_valid() {
            return Err(AzSceneCodecError::InvalidComponentTargetId { entity });
        }
        if previous_target.is_some_and(|previous: uuid::Uuid| previous >= native_type_id) {
            return Err(AzSceneCodecError::NonCanonicalComponentTargetOrder { entity });
        }
        if !local_ids.insert(component_id.value()) {
            return Err(AzSceneCodecError::DuplicateComponentTargetId {
                entity,
                component_id: component_id.value(),
            });
        }
        previous_target = Some(native_type_id);
        component_targets.push(AzSceneComponentTarget {
            native_type_id,
            component_id,
        });
    }
    Ok(component_targets)
}

fn type_table(
    world: &DynamicWorld,
    registry: &TypeRegistry,
) -> Result<Vec<String>, AzSceneCodecError> {
    let mut table = BTreeSet::new();
    collect_type_paths(&world.resources, registry, &mut table)?;
    for entity in &world.entities {
        collect_type_paths(&entity.components, registry, &mut table)?;
    }
    write_count(&mut std::io::sink(), "types", table.len(), MAX_TYPE_COUNT)?;
    Ok(table.into_iter().collect())
}

fn collect_type_paths(
    values: &[Box<dyn PartialReflect>],
    registry: &TypeRegistry,
    table: &mut BTreeSet<String>,
) -> Result<(), AzSceneCodecError> {
    for value in values {
        let type_path = reflected_type_path(value.as_partial_reflect())?;
        let registration = registry.get_with_type_path(type_path).ok_or_else(|| {
            AzSceneCodecError::UnregisteredType {
                type_path: type_path.to_owned(),
            }
        })?;
        let canonical = registration.type_info().type_path();
        if canonical != type_path {
            return Err(AzSceneCodecError::NonCanonicalTypePath {
                encoded: type_path.to_owned(),
                canonical: canonical.to_owned(),
            });
        }
        table.insert(type_path.to_owned());
    }
    Ok(())
}

fn sorted_values(
    values: &[Box<dyn PartialReflect>],
) -> Result<Vec<(&str, &dyn PartialReflect)>, AzSceneCodecError> {
    let mut sorted = values
        .iter()
        .map(|value| {
            let value = value.as_partial_reflect();
            Ok((reflected_type_path(value)?, value))
        })
        .collect::<Result<Vec<_>, AzSceneCodecError>>()?;
    sorted.sort_by_key(|(type_path, _)| *type_path);
    if sorted.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(AzSceneCodecError::DuplicateValueType {
            type_path: sorted
                .windows(2)
                .find(|pair| pair[0].0 == pair[1].0)
                .map(|pair| pair[0].0.to_owned())
                .unwrap_or_default(),
        });
    }
    Ok(sorted)
}

fn reflected_type_path(value: &dyn PartialReflect) -> Result<&str, AzSceneCodecError> {
    Ok(value
        .get_represented_type_info()
        .ok_or(AzSceneCodecError::MissingRepresentedType)?
        .type_path())
}

fn read_source_scopes(
    reader: &mut impl Read,
    count: usize,
) -> Result<Vec<AzSceneSourceScopeMetadata>, AzSceneCodecError> {
    let mut scopes = Vec::with_capacity(count);
    for expected in 0..count {
        let expected = u32::try_from(expected).map_err(|_| AzSceneCodecError::LengthOverflow {
            kind: "source scopes",
            len: expected,
        })?;
        let found = read_u32(reader)?;
        if found != expected {
            return Err(AzSceneCodecError::NonCanonicalSourceScopeOrder { expected, found });
        }
        let parent = read_u32(reader)?;
        scopes.push(AzSceneSourceScopeMetadata {
            parent: (parent != NO_PARENT).then(|| LocalEntityScopeId::new(parent)),
        });
    }
    validate_source_scopes(&scopes)?;
    Ok(scopes)
}

fn validate_source_scopes(scopes: &[AzSceneSourceScopeMetadata]) -> Result<(), AzSceneCodecError> {
    if scopes.is_empty() {
        return Err(AzSceneCodecError::MissingRootSourceScope);
    }
    for (index, scope) in scopes.iter().enumerate() {
        let scope_id = u32::try_from(index).unwrap_or(u32::MAX);
        match (scope_id, scope.parent) {
            (0, None) => {}
            (0, Some(parent)) => {
                return Err(AzSceneCodecError::InvalidSourceScopeParent {
                    source_scope: scope_id,
                    parent: parent.value(),
                });
            }
            (_, Some(parent)) if parent.value() < scope_id => {}
            (_, Some(parent)) => {
                return Err(AzSceneCodecError::InvalidSourceScopeParent {
                    source_scope: scope_id,
                    parent: parent.value(),
                });
            }
            (_, None) => {
                return Err(AzSceneCodecError::MissingSourceScopeParent {
                    source_scope: scope_id,
                });
            }
        }
    }
    Ok(())
}

fn validate_metadata(asset: &AzSceneAsset) -> Result<(), AzSceneCodecError> {
    if asset.metadata.entities.len() != asset.dynamic_world.entities.len() {
        return Err(AzSceneCodecError::MetadataEntityCount {
            metadata: asset.metadata.entities.len(),
            entities: asset.dynamic_world.entities.len(),
        });
    }
    write_count(
        &mut std::io::sink(),
        "entities",
        asset.dynamic_world.entities.len(),
        MAX_ENTITY_COUNT,
    )?;
    validate_metadata_entries(&asset.metadata)
}

fn validate_metadata_entries(metadata: &AzSceneMetadata) -> Result<(), AzSceneCodecError> {
    validate_source_scopes(&metadata.source_scopes)?;
    let source_scope_count = u32::try_from(metadata.source_scopes.len()).unwrap_or(u32::MAX);
    let mut aliases = BTreeSet::new();
    let mut source_entity_ids = BTreeSet::new();
    let entity_count = u32::try_from(metadata.entities.len()).unwrap_or(u32::MAX);
    for (index, entity) in metadata.entities.iter().enumerate() {
        if entity.source_alias.is_empty() || !aliases.insert(&entity.source_alias) {
            return Err(AzSceneCodecError::InvalidSourceAlias {
                source_alias: entity.source_alias.clone(),
            });
        }
        let entity_index = u32::try_from(index).unwrap_or(u32::MAX);
        if entity.source_scope.value() >= source_scope_count {
            return Err(AzSceneCodecError::InvalidEntitySourceScope {
                entity: entity_index,
                source_scope: entity.source_scope.value(),
                source_scope_count,
            });
        }
        if let Some(source_entity_id) = entity.source_entity_id {
            if source_entity_id.is_invalid() {
                return Err(AzSceneCodecError::InvalidSourceEntityId {
                    entity: entity_index,
                });
            }
            if !source_entity_ids.insert((entity.source_scope, source_entity_id.value())) {
                return Err(AzSceneCodecError::DuplicateSourceEntityId {
                    entity: entity_index,
                    source_entity_id,
                });
            }
        }
        if let Some(parent) = entity.parent {
            let local = u32::try_from(index).unwrap_or(u32::MAX);
            if parent.value() >= entity_count || parent.value() == local {
                return Err(AzSceneCodecError::InvalidParent {
                    entity: local,
                    parent: parent.value(),
                    entity_count,
                });
            }
        }
        let mut previous_type = None;
        let mut local_ids = BTreeSet::new();
        for target in &entity.component_targets {
            if !target.component_id.is_valid() {
                return Err(AzSceneCodecError::InvalidComponentTargetId {
                    entity: u32::try_from(index).unwrap_or(u32::MAX),
                });
            }
            if previous_type.is_some_and(|previous: uuid::Uuid| previous >= target.native_type_id) {
                return Err(AzSceneCodecError::NonCanonicalComponentTargetOrder {
                    entity: u32::try_from(index).unwrap_or(u32::MAX),
                });
            }
            if !local_ids.insert(target.component_id.value()) {
                return Err(AzSceneCodecError::DuplicateComponentTargetId {
                    entity: u32::try_from(index).unwrap_or(u32::MAX),
                    component_id: target.component_id.value(),
                });
            }
            previous_type = Some(target.native_type_id);
        }
    }
    validate_hierarchy(&metadata.entities)
}

fn validate_used_types(
    type_table: &[String],
    used_types: &BTreeSet<u32>,
) -> Result<(), AzSceneCodecError> {
    if used_types.len() == type_table.len() {
        return Ok(());
    }
    let unused = type_table
        .iter()
        .enumerate()
        .find(|(index, _)| !used_types.contains(&u32::try_from(*index).unwrap_or(u32::MAX)))
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    Err(AzSceneCodecError::UnusedTypeTableEntry { type_path: unused })
}

fn validate_hierarchy(metadata: &[AzSceneEntityMetadata]) -> Result<(), AzSceneCodecError> {
    for start in 0..metadata.len() {
        let mut seen = BTreeSet::new();
        let mut cursor = Some(u32::try_from(start).unwrap_or(u32::MAX));
        while let Some(local) = cursor {
            if !seen.insert(local) {
                return Err(AzSceneCodecError::HierarchyCycle { entity: local });
            }
            cursor = metadata
                .get(usize::try_from(local).unwrap_or(usize::MAX))
                .and_then(|entity| entity.parent)
                .map(LocalEntityId::value);
        }
    }
    Ok(())
}

fn validate_dependency_paths(paths: &[String]) -> Result<(), AzSceneCodecError> {
    for path in paths {
        if path.is_empty() || path.contains('\\') || path.chars().any(char::is_control) {
            return Err(AzSceneCodecError::InvalidDependencyPath { path: path.clone() });
        }
    }
    Ok(())
}

fn write_value(writer: &mut impl Write, value: &EncodedValue) -> Result<(), AzSceneCodecError> {
    write_u32(writer, value.type_index)?;
    write_bounded_bytes(writer, "payload", &value.payload, MAX_PAYLOAD_BYTES)
}

fn read_sorted_table(
    reader: &mut impl Read,
    kind: &'static str,
    count: usize,
    max_bytes: u32,
) -> Result<Vec<String>, AzSceneCodecError> {
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        let value = read_bounded_string(reader, kind, max_bytes)?;
        if values.last().is_some_and(|previous| previous >= &value) {
            return Err(AzSceneCodecError::NonCanonicalTable { kind });
        }
        values.push(value);
    }
    Ok(values)
}

fn write_count(
    writer: &mut impl Write,
    kind: &'static str,
    len: usize,
    max: u32,
) -> Result<(), AzSceneCodecError> {
    let count = u32::try_from(len).map_err(|_| AzSceneCodecError::LengthOverflow { kind, len })?;
    if count > max {
        return Err(AzSceneCodecError::CountTooLarge { kind, count, max });
    }
    write_u32(writer, count)?;
    Ok(())
}

fn read_count(
    reader: &mut impl Read,
    kind: &'static str,
    max: u32,
) -> Result<usize, AzSceneCodecError> {
    let count = read_u32(reader)?;
    if count > max {
        return Err(AzSceneCodecError::CountTooLarge { kind, count, max });
    }
    usize::try_from(count).map_err(|_| AzSceneCodecError::PlatformCount { kind, count })
}

fn write_bounded_string(
    writer: &mut impl Write,
    kind: &'static str,
    value: &str,
    max: u32,
) -> Result<(), AzSceneCodecError> {
    write_bounded_bytes(writer, kind, value.as_bytes(), max)
}

fn read_bounded_string(
    reader: &mut impl Read,
    kind: &'static str,
    max: u32,
) -> Result<String, AzSceneCodecError> {
    Ok(String::from_utf8(read_bounded_bytes(reader, kind, max)?)?)
}

fn write_bounded_bytes(
    writer: &mut impl Write,
    kind: &'static str,
    bytes: &[u8],
    max: u32,
) -> Result<(), AzSceneCodecError> {
    let len = u32::try_from(bytes.len()).map_err(|_| AzSceneCodecError::LengthOverflow {
        kind,
        len: bytes.len(),
    })?;
    if len > max {
        return Err(AzSceneCodecError::ChunkTooLarge { kind, len, max });
    }
    write_u32(writer, len)?;
    writer.write_all(bytes)?;
    Ok(())
}

fn read_bounded_bytes(
    reader: &mut impl Read,
    kind: &'static str,
    max: u32,
) -> Result<Vec<u8>, AzSceneCodecError> {
    let len = read_u32(reader)?;
    if len > max {
        return Err(AzSceneCodecError::ChunkTooLarge { kind, len, max });
    }
    let len =
        usize::try_from(len).map_err(|_| AzSceneCodecError::PlatformCount { kind, count: len })?;
    let mut bytes = vec![0_u8; len];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn discard_bounded_bytes(
    reader: &mut impl Read,
    kind: &'static str,
    max: u32,
) -> Result<(), AzSceneCodecError> {
    let len = read_u32(reader)?;
    if len > max {
        return Err(AzSceneCodecError::ChunkTooLarge { kind, len, max });
    }
    let copied = std::io::copy(&mut reader.take(u64::from(len)), &mut std::io::sink())?;
    if copied != u64::from(len) {
        return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into());
    }
    Ok(())
}

fn write_u8(writer: &mut impl Write, value: u8) -> Result<(), std::io::Error> {
    writer.write_all(&[value])
}

fn read_u8(reader: &mut impl Read) -> Result<u8, std::io::Error> {
    let mut bytes = [0_u8; 1];
    reader.read_exact(&mut bytes)?;
    Ok(bytes[0])
}

fn write_u32(writer: &mut impl Write, value: u32) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u32(reader: &mut impl Read) -> Result<u32, std::io::Error> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn write_u64(writer: &mut impl Write, value: u64) -> Result<(), std::io::Error> {
    writer.write_all(&value.to_le_bytes())
}

fn read_u64(reader: &mut impl Read) -> Result<u64, std::io::Error> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_trailer(reader: &mut impl Read) -> Result<(), AzSceneCodecError> {
    let mut trailing = [0_u8; 1];
    match reader.read(&mut trailing)? {
        0 => Ok(()),
        _ => Err(AzSceneCodecError::TrailingAssetBytes),
    }
}

#[derive(Default)]
struct RejectPathLoads {
    attempted: bool,
}

impl LoadFromPath for RejectPathLoads {
    fn load_from_path_erased(
        &mut self,
        type_id: std::any::TypeId,
        _path: bevy::asset::AssetPath<'static>,
    ) -> UntypedHandle {
        self.attempted = true;
        UntypedHandle::Uuid {
            type_id,
            uuid: AssetId::<()>::DEFAULT_UUID,
        }
    }
}

/// Bevy loader for `*.scn.bin` products.
#[derive(Debug, TypePath)]
pub struct AzSceneAssetLoader {
    type_registry: TypeRegistryArc,
}

impl FromWorld for AzSceneAssetLoader {
    fn from_world(world: &mut World) -> Self {
        Self {
            type_registry: world.resource::<AppTypeRegistry>().0.clone(),
        }
    }
}

impl AssetLoader for AzSceneAssetLoader {
    type Asset = AzSceneAsset;
    type Settings = ();
    type Error = AzSceneCodecError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 16 * 1024];
        loop {
            let read = reader.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            if bytes.len().saturating_add(read) > MAX_FILE_BYTES {
                return Err(AzSceneCodecError::FileTooLarge {
                    len: bytes.len().saturating_add(read),
                    max: MAX_FILE_BYTES,
                });
            }
            bytes.extend_from_slice(&chunk[..read]);
        }
        read_scene_asset_with_loader(Cursor::new(bytes), &self.type_registry.read(), load_context)
    }

    fn extensions(&self) -> &[&str] {
        &[AZSCENE_PRODUCT_EXTENSION]
    }
}

#[derive(Debug, Error)]
pub enum AzSceneCodecError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("bad AZSCENE magic: {found:?}")]
    BadMagic { found: [u8; 8] },
    #[error("unsupported AZSCENE version {version}, expected {expected}")]
    UnsupportedVersion { version: u32, expected: u32 },
    #[error("AZSCENE {kind} length {len} exceeds u32")]
    LengthOverflow { kind: &'static str, len: usize },
    #[error("AZSCENE {kind} count {count} exceeds limit {max}")]
    CountTooLarge {
        kind: &'static str,
        count: u32,
        max: u32,
    },
    #[error("AZSCENE {kind} count {count} exceeds platform capacity")]
    PlatformCount { kind: &'static str, count: u32 },
    #[error("AZSCENE {kind} chunk is {len} bytes, limit is {max}")]
    ChunkTooLarge {
        kind: &'static str,
        len: u32,
        max: u32,
    },
    #[error("AZSCENE file is {len} bytes, limit is {max}")]
    FileTooLarge { len: usize, max: usize },
    #[error("reflected AZSCENE value has no represented type")]
    MissingRepresentedType,
    #[error("AZSCENE type `{type_path}` is not registered")]
    UnregisteredType { type_path: String },
    #[error("AZSCENE encoded type path `{encoded}` is not canonical `{canonical}`")]
    NonCanonicalTypePath { encoded: String, canonical: String },
    #[error("AZSCENE type `{type_path}` is absent from the type table")]
    TypeNotInTable { type_path: String },
    #[error("AZSCENE type index {index} is outside {type_count} entries")]
    InvalidTypeIndex { index: u32, type_count: usize },
    #[error("AZSCENE has duplicate value type `{type_path}` in one record")]
    DuplicateValueType { type_path: String },
    #[error("AZSCENE {kind} table is not strictly lexically sorted and unique")]
    NonCanonicalTable { kind: &'static str },
    #[error("AZSCENE type table contains unused `{type_path}`")]
    UnusedTypeTableEntry { type_path: String },
    #[error("AZSCENE entity order expected local id {expected}, found {found}")]
    NonCanonicalEntityOrder { expected: u32, found: u32 },
    #[error("AZSCENE source scope order expected local id {expected}, found {found}")]
    NonCanonicalSourceScopeOrder { expected: u32, found: u32 },
    #[error("AZSCENE source scope table has no root scope")]
    MissingRootSourceScope,
    #[error("AZSCENE source scope {source_scope} has no parent")]
    MissingSourceScopeParent { source_scope: u32 },
    #[error("AZSCENE source scope {source_scope} has invalid parent {parent}")]
    InvalidSourceScopeParent { source_scope: u32, parent: u32 },
    #[error(
        "AZSCENE entity {entity} has source scope {source_scope} outside {source_scope_count} scopes"
    )]
    InvalidEntitySourceScope {
        entity: u32,
        source_scope: u32,
        source_scope_count: u32,
    },
    #[error("AZSCENE entity {entity} components are not strictly TypePath ordered")]
    NonCanonicalComponentOrder { entity: u32 },
    #[error("AZSCENE entity {entity} component targets are not strictly native-type ordered")]
    NonCanonicalComponentTargetOrder { entity: u32 },
    #[error("AZSCENE entity {entity} contains an invalid component target id")]
    InvalidComponentTargetId { entity: u32 },
    #[error("AZSCENE entity {entity} contains duplicate component target id {component_id:#x}")]
    DuplicateComponentTargetId { entity: u32, component_id: u64 },
    #[error("invalid AZSCENE local entity id {local_id}")]
    InvalidLocalEntity { local_id: u32 },
    #[error("invalid or duplicate AZSCENE source alias `{source_alias}`")]
    InvalidSourceAlias { source_alias: String },
    #[error("AZSCENE entity {entity} contains an invalid authored source entity id")]
    InvalidSourceEntityId { entity: u32 },
    #[error("AZSCENE entity {entity} duplicates authored source entity id {source_entity_id:?}")]
    DuplicateSourceEntityId {
        entity: u32,
        source_entity_id: EntityId,
    },
    #[error("AZSCENE {kind} option has invalid tag {tag}")]
    InvalidOptionTag { kind: &'static str, tag: u8 },
    #[error("AZSCENE entity {entity} has invalid parent {parent} for {entity_count} entities")]
    InvalidParent {
        entity: u32,
        parent: u32,
        entity_count: u32,
    },
    #[error("AZSCENE hierarchy contains a cycle through entity {entity}")]
    HierarchyCycle { entity: u32 },
    #[error("AZSCENE metadata has {metadata} entities but the dynamic world has {entities}")]
    MetadataEntityCount { metadata: usize, entities: usize },
    #[error("invalid AZSCENE dependency path `{path}`")]
    InvalidDependencyPath { path: String },
    #[error("AZSCENE path handles require a Bevy asset load context")]
    AssetLoaderRequired,
    #[error("AZSCENE dependency ledger mismatch: declared {declared:?}, observed {observed:?}")]
    DependencyLedgerMismatch {
        declared: Vec<String>,
        observed: Vec<String>,
    },
    #[error("AZSCENE reflected value serialization error for `{type_path}`: {source}")]
    ReflectedValueSerialization {
        type_path: String,
        source: serde_value::SerializerError,
    },
    #[error("AZSCENE reflected value deserialization error for `{type_path}`: {source}")]
    ReflectedValueDeserialization {
        type_path: String,
        source: serde_value::DeserializerError,
    },
    #[error("invalid AZSCENE reflected value for `{type_path}`: {reason}")]
    InvalidReflectedValue { type_path: String, reason: String },
    #[error("AZSCENE postcard error for `{type_path}`: {source}")]
    Postcard {
        type_path: String,
        source: postcard::Error,
    },
    #[error("AZSCENE payload for `{type_path}` has {trailing} trailing bytes")]
    TrailingPayloadBytes { type_path: String, trailing: usize },
    #[error("AZSCENE text is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("AZSCENE has trailing bytes after its records")]
    TrailingAssetBytes,
}
