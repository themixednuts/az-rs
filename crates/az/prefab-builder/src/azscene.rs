//! Typed Prefab source processing into the engine scene product.

use std::{
    any::TypeId,
    collections::{BTreeMap, BTreeSet},
};

use az_core::component::ComponentLoweringRegistration;
use az_core::{DiagnosticSeverity, EntityId, ValidationTypeData};
use az_gem_contract::Registry;
use az_prefab::{
    EntityAlias, OverrideOperation, PrefabBuildError, PrefabCodec, PrefabCodecError,
    PrefabCollection, PrefabDocument, PrefabInstance, PrefabProductPolicy, PrefabRegistry,
    PrefabType, PrefabTypeData, SparseValue, TypedOverrideAction as OverrideAction,
    TypedPrefabSemantics, TypedPrefabSemanticsError,
};
use az_scene::{
    AzSceneAsset, AzSceneCodecError, AzSceneComponentTarget, AzSceneEntityMetadata,
    AzSceneEntityOrderError, AzSceneMetadata, AzSceneSourceScopeMetadata, LocalEntityId,
    LocalEntityScopeId, encode_scene_asset,
};
use bevy::{
    ecs::{
        hierarchy::ChildOf, reflect::AppTypeRegistry, template::SceneEntityReferences, world::World,
    },
    reflect::{PartialReflect, ReflectMut, TypeRegistry, structs::DynamicStruct},
    world_serialization::DynamicWorldBuilder,
};
use thiserror::Error;
use tracing::{info, instrument};

const PREFAB_SOURCE_SUFFIX: &str = ".prefab.ron";

/// Result of the additive Phase 5 processing path. No supported build rule
/// selects this path until the workflow cutover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledAzScene {
    pub product_path: String,
    /// Additional lookup paths for the same canonical product.
    pub catalog_aliases: Vec<String>,
    pub bytes: Vec<u8>,
    /// Nested Prefab sources that re-fingerprint this product.
    pub source_dependencies: Vec<String>,
    /// Bevy path handles embedded in the AZSCENE dependency table.
    pub path_dependencies: Vec<String>,
    /// Canonical runtime products referenced by reflected asset values.
    pub asset_dependencies: Vec<az_asset_builder::AssetId>,
}

/// The engine's reflected Prefab types, applied directly to a Bevy registry.
///
/// Direct application is the sanctioned shape here and only here: the target is
/// Bevy's [`AppTypeRegistry`], which is not a composed registry at all, so there
/// is no attribution to carry, no key to collide on, and nothing for a compose
/// report to say (asset-contract ticket 014, D7's carve-out). Its former
/// sibling `engine_lowerings()` was a different animal wearing the same coat —
/// component lowerings *are* a composed registry — and it is gone: the engine's
/// lowerings arrive through the `runtime` bundle, attributed like everything
/// else.
#[must_use]
pub fn engine_prefab_type_registry() -> AppTypeRegistry {
    let registry = AppTypeRegistry::default();
    {
        let mut registry = registry.write();
        az_prefab::register_core_prefab_types(&mut registry);
        az_transform::register_transform_prefab_types(&mut registry);
        az_render::register_render_prefab_types(&mut registry);
        az_framework::register_framework_prefab_types(&mut registry);
        registry.register::<bevy::ecs::entity::Entity>();
        registry.register::<ChildOf>();
    }
    registry
}

/// One host's composed component lowerings, in composition order.
///
/// There is nothing to merge any more. The engine's own adapters are composed
/// entries like a gem's, so this is the whole set rather than the composed half
/// of it. That is what makes the one-lowering-per-type rule mean something: a
/// second adapter for a component the engine already lowers is now a compose
/// error naming both culprits, where before the engine's half arrived outside
/// composition and could not collide with anything.
#[must_use]
pub fn lowerings(
    composed: &Registry<ComponentLoweringRegistration>,
) -> Vec<ComponentLoweringRegistration> {
    composed.entries().copied().collect()
}

/// Composes the engine registrations with a host's composed Prefab types.
///
/// Entries arrive in composition order and are applied in that order. Nothing
/// is re-sorted or de-duplicated here: the composer already rejected a
/// duplicate type path, naming both contributions, before this runs.
#[must_use]
pub fn prefab_type_registry(prefab_types: &Registry<PrefabType>) -> AppTypeRegistry {
    let registry = engine_prefab_type_registry();
    {
        let mut registry = registry.write();
        for prefab_type in prefab_types.entries() {
            prefab_type.apply(&mut registry);
        }
    }
    registry
}

/// Canonical catalog path for a processed Prefab scene product.
#[must_use]
pub fn azscene_product_path(source_path: &str) -> String {
    let normalized = source_path.replace('\\', "/");
    let stem = normalized
        .strip_suffix(PREFAB_SOURCE_SUFFIX)
        .unwrap_or(normalized.as_str());
    if stem == "prefabs" || stem.starts_with("prefabs/") {
        format!("{stem}.scn.bin")
    } else {
        format!("prefabs/{stem}.scn.bin")
    }
}

// The codec and the decoded document borrow from this type-registry read guard for
// the rest of the function, so the guard cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
/// Decodes, migrates, validates, flattens, constructs, captures, and encodes
/// one typed Prefab source.
///
/// `resolve_source` receives normalized nested source paths and returns their
/// UTF-8 Prefab source text. The root source is supplied separately and is not
/// requested from the resolver.
///
/// # Errors
///
/// Returns [`PrefabAzSceneProcessError::Registry`] if the typed Prefab registry
/// cannot be built, [`PrefabAzSceneProcessError::Source`] if the root source
/// fails decode, migration or validation, and the resolve, hierarchy, override
/// and component variants for a nested source that cannot be resolved, a parent
/// or override target that does not exist, an alias or authored entity id that
/// repeats, or a component that fails construction or validation. See
/// [`PrefabAzSceneProcessError`] for the full set.
#[instrument(skip_all, fields(source_path))]
pub fn process_prefab_to_azscene(
    source_path: &str,
    source: &str,
    app_registry: &AppTypeRegistry,
    lowerings: &[ComponentLoweringRegistration],
    mut resolve_source: impl FnMut(&str) -> Result<String, String>,
) -> Result<CompiledAzScene, PrefabAzSceneProcessError> {
    let registry = app_registry.read();
    let registry = &*registry;
    let codec = PrefabCodec::new(registry)?;
    let document = codec
        .decode(source)
        .map_err(|source| PrefabAzSceneProcessError::Source {
            source_path: source_path.to_owned(),
            source,
        })?;
    let nested_codec = PrefabCodec::new(registry)?;
    process_prefab_document_to_azscene(
        source_path,
        &document,
        app_registry,
        registry,
        lowerings,
        |nested_path| {
            let nested_source = resolve_source(nested_path)?;
            nested_codec
                .decode(&nested_source)
                .map_err(|error| error.to_string())
        },
    )
}

// The codec and the decoded document borrow from this type-registry read guard for
// the rest of the function, so the guard cannot be released any earlier.
#[allow(clippy::significant_drop_tightening)]
/// Compiles every member of one aggregate Prefab source while resolving
/// member-to-member references without a second source file.
///
/// `resolve_source` is consulted only for nested Prefabs outside the
/// collection. Returned entries retain the collection's stable product
/// sub-IDs.
///
/// # Errors
///
/// Returns any error [`process_prefab_to_azscene`] returns, for the first
/// collection entry that fails.
pub fn process_prefab_collection_to_azscenes(
    collection: &PrefabCollection,
    app_registry: &AppTypeRegistry,
    lowerings: &[ComponentLoweringRegistration],
    mut resolve_source: impl FnMut(&str) -> Result<String, String>,
) -> Result<Vec<(u32, CompiledAzScene)>, PrefabAzSceneProcessError> {
    let registry = app_registry.read();
    let nested_codec = PrefabCodec::new(&registry)?;
    collection
        .iter()
        .map(|(sub_id, entry)| {
            process_prefab_document_to_azscene(
                entry.source_path().as_str(),
                entry.document(),
                app_registry,
                &registry,
                lowerings,
                |nested_path| {
                    if let Some(nested) = collection.entry_by_source_path(nested_path) {
                        return Ok(nested.document().clone());
                    }
                    let nested_source = resolve_source(nested_path)?;
                    nested_codec
                        .decode(&nested_source)
                        .map_err(|error| error.to_string())
                },
            )
            .map(|compiled| (sub_id, compiled))
        })
        .collect()
}

fn process_prefab_document_to_azscene(
    source_path: &str,
    document: &PrefabDocument,
    app_registry: &AppTypeRegistry,
    registry: &TypeRegistry,
    lowerings: &[ComponentLoweringRegistration],
    mut resolve_document: impl FnMut(&str) -> Result<PrefabDocument, String>,
) -> Result<CompiledAzScene, PrefabAzSceneProcessError> {
    let prefab_registry = PrefabRegistry::try_new(registry)?;
    let catalog_aliases = document
        .catalog_aliases
        .iter()
        .map(|alias| alias.as_str().to_owned())
        .collect();
    let mut context = ResolveContext {
        resolve_document: &mut resolve_document,
        registry,
        prefab_registry,
        stack: vec![source_path.to_owned()],
        source_dependencies: BTreeSet::new(),
    };
    let resolved = context.resolve_document(source_path, document)?;
    validate_resolved_hierarchy(source_path, &resolved)?;
    let ConstructedPrefabWorld {
        world,
        entity_order,
        entity_metadata,
        source_scopes,
        source_only_component_types,
    } = construct_world(source_path, registry, app_registry, lowerings, &resolved)?;
    let mut dynamic_world = DynamicWorldBuilder::from_world(&world, registry)
        .extract_entities(entity_order.iter().copied())
        .build();
    // Hierarchy is encoded as local metadata and re-established after all
    // destination entities exist. Keeping relationship components out of the
    // payload avoids double-running relationship hooks at materialization.
    let child_of = TypeId::of::<ChildOf>();
    let mut omitted_source_only_components = 0_usize;
    for entity in &mut dynamic_world.entities {
        entity.components.retain(|component| {
            let Some(type_info) = component.get_represented_type_info() else {
                return true;
            };
            if type_info.type_id() == child_of {
                return false;
            }
            let source_only = source_only_component_types
                .get(&entity.entity)
                .is_some_and(|types| types.contains(&type_info.type_id()));
            omitted_source_only_components += usize::from(source_only);
            !source_only
        });
    }
    let asset = AzSceneAsset::new_in_entity_order(
        dynamic_world,
        &entity_order,
        AzSceneMetadata {
            source_scopes,
            entities: entity_metadata,
        },
    )?;
    let encoded = encode_scene_asset(&asset, registry)?;
    info!(
        source_dependencies = context.source_dependencies.len(),
        path_dependencies = encoded.dependencies.len(),
        asset_dependencies = encoded.asset_dependencies.len(),
        omitted_source_only_components,
        bytes = encoded.bytes.len(),
        version = az_scene::AZSCENE_FORMAT_VERSION,
        "processed typed Prefab to AZSCENE"
    );
    Ok(CompiledAzScene {
        product_path: azscene_product_path(source_path),
        catalog_aliases,
        bytes: encoded.bytes,
        source_dependencies: context.source_dependencies.into_iter().collect(),
        path_dependencies: encoded.dependencies,
        asset_dependencies: encoded.asset_dependencies,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntityKey {
    instance_chain: Vec<String>,
    entity: String,
}

impl PartialOrd for EntityKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for EntityKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.source_alias()
            .cmp(&other.source_alias())
            .then_with(|| self.instance_chain.cmp(&other.instance_chain))
            .then_with(|| self.entity.cmp(&other.entity))
    }
}

impl EntityKey {
    fn direct(alias: &EntityAlias) -> Self {
        Self {
            instance_chain: Vec::new(),
            entity: alias.as_str().to_owned(),
        }
    }

    fn prefixed(mut self, instance: &str) -> Self {
        self.instance_chain.insert(0, instance.to_owned());
        self
    }

    fn source_alias(&self) -> String {
        self.instance_chain
            .iter()
            .chain(std::iter::once(&self.entity))
            .map(|segment| escape_alias_segment(segment))
            .collect::<Vec<_>>()
            .join("/")
    }
}

#[derive(Debug, Clone)]
struct ResolvedEntity {
    source_entity_id: Option<EntityId>,
    parent: Option<EntityKey>,
    components: BTreeMap<String, SparseValue>,
}

type ResolvedEntities = BTreeMap<EntityKey, ResolvedEntity>;

struct ResolveContext<'a, F> {
    resolve_document: &'a mut F,
    registry: &'a TypeRegistry,
    prefab_registry: PrefabRegistry<'a>,
    stack: Vec<String>,
    source_dependencies: BTreeSet<String>,
}

impl<F> ResolveContext<'_, F>
where
    F: FnMut(&str) -> Result<PrefabDocument, String>,
{
    fn resolve_document(
        &mut self,
        source_path: &str,
        document: &PrefabDocument,
    ) -> Result<ResolvedEntities, PrefabAzSceneProcessError> {
        TypedPrefabSemantics::validate_local(document, self.registry).map_err(|source| {
            PrefabAzSceneProcessError::TypedValidation {
                source_path: source_path.to_owned(),
                source,
            }
        })?;
        validate_local_document(source_path, document)?;
        let mut result = document
            .entities
            .iter()
            .map(|(alias, entity)| {
                (
                    EntityKey::direct(alias),
                    ResolvedEntity {
                        source_entity_id: entity.entity_id,
                        parent: entity.parent.as_ref().map(EntityKey::direct),
                        components: entity.components.clone(),
                    },
                )
            })
            .collect::<ResolvedEntities>();

        for (instance_alias, instance) in &document.instances {
            let nested_path = instance.source.as_str();
            if let Some(start) = self.stack.iter().position(|path| path == nested_path) {
                let mut chain = self.stack[start..].to_vec();
                chain.push(nested_path.to_owned());
                return Err(PrefabAzSceneProcessError::SourceCycle { chain });
            }
            self.source_dependencies.insert(nested_path.to_owned());
            let nested_document = (self.resolve_document)(nested_path).map_err(|message| {
                PrefabAzSceneProcessError::ResolveSource {
                    source_path: source_path.to_owned(),
                    instance_alias: instance_alias.as_str().to_owned(),
                    nested_path: nested_path.to_owned(),
                    message,
                }
            })?;
            self.stack.push(nested_path.to_owned());
            let mut nested = self.resolve_document(nested_path, &nested_document)?;
            self.stack.pop();
            apply_overrides(
                source_path,
                instance_alias.as_str(),
                &mut nested,
                instance,
                &self.prefab_registry,
            )?;

            let instance_name = instance_alias.as_str();
            let direct_parent = instance.parent.as_ref().map(EntityKey::direct);
            for (key, mut entity) in nested {
                let prefixed_key = key.clone().prefixed(instance_name);
                entity.parent = entity
                    .parent
                    .map(|parent| parent.prefixed(instance_name))
                    .or_else(|| direct_parent.clone());
                if result.insert(prefixed_key.clone(), entity).is_some() {
                    return Err(PrefabAzSceneProcessError::DuplicateFlattenedAlias {
                        source_path: source_path.to_owned(),
                        alias: prefixed_key.source_alias(),
                    });
                }
            }
        }
        Ok(result)
    }
}

fn validate_local_document(
    source_path: &str,
    document: &PrefabDocument,
) -> Result<(), PrefabAzSceneProcessError> {
    for (alias, entity) in &document.entities {
        if let Some(parent) = &entity.parent
            && !document.entities.contains_key(parent)
        {
            return Err(PrefabAzSceneProcessError::MissingParent {
                source_path: source_path.to_owned(),
                entity_alias: alias.as_str().to_owned(),
                parent_alias: parent.as_str().to_owned(),
            });
        }
    }
    for (alias, instance) in &document.instances {
        if let Some(parent) = &instance.parent
            && !document.entities.contains_key(parent)
        {
            return Err(PrefabAzSceneProcessError::MissingParent {
                source_path: source_path.to_owned(),
                entity_alias: alias.as_str().to_owned(),
                parent_alias: parent.as_str().to_owned(),
            });
        }
    }
    Ok(())
}

fn apply_overrides(
    source_path: &str,
    instance_alias: &str,
    entities: &mut ResolvedEntities,
    instance: &PrefabInstance,
    registry: &PrefabRegistry<'_>,
) -> Result<(), PrefabAzSceneProcessError> {
    for operation in &instance.overrides {
        let key = EntityKey {
            instance_chain: operation
                .target
                .instance_chain
                .iter()
                .map(|alias| alias.as_str().to_owned())
                .collect(),
            entity: operation.target.entity.as_str().to_owned(),
        };
        let entity = entities.get_mut(&key).ok_or_else(|| {
            PrefabAzSceneProcessError::OverrideTarget(Box::new(OverrideTargetFailure {
                source_path: source_path.to_owned(),
                instance_alias: instance_alias.to_owned(),
                entity_alias: key.source_alias(),
                component: operation.target.component.clone(),
                path: operation.target.path.to_string(),
                message: "entity does not exist after nested resolution".to_owned(),
            }))
        })?;
        apply_override(
            source_path,
            instance_alias,
            &key,
            entity,
            operation,
            registry,
        )?;
    }
    Ok(())
}

fn apply_override(
    source_path: &str,
    instance_alias: &str,
    key: &EntityKey,
    entity: &mut ResolvedEntity,
    operation: &OverrideOperation,
    registry: &PrefabRegistry<'_>,
) -> Result<(), PrefabAzSceneProcessError> {
    let error = |message: String| {
        PrefabAzSceneProcessError::OverrideTarget(Box::new(OverrideTargetFailure {
            source_path: source_path.to_owned(),
            instance_alias: instance_alias.to_owned(),
            entity_alias: key.source_alias(),
            component: operation.target.component.clone(),
            path: operation.target.path.to_string(),
            message,
        }))
    };
    let component_key = registry
        .resolve_prefab_type_path(&operation.target.component)
        .map_err(|source| error(source.to_string()))?
        .prefab
        .tag
        .to_owned();
    if !entity.components.contains_key(&component_key) {
        return Err(error("component does not exist".to_owned()));
    }
    if operation.target.path.is_root() {
        return match &operation.action {
            OverrideAction::Set(value) => {
                entity.components.insert(component_key, value.clone());
                Ok(())
            }
            OverrideAction::Clear => {
                entity.components.remove(&component_key);
                Ok(())
            }
            action => {
                let component = entity
                    .components
                    .get_mut(&component_key)
                    .ok_or_else(|| error("component does not exist".to_owned()))?;
                apply_collection_action(component.value_mut(), action).map_err(error)
            }
        };
    }

    let component = entity
        .components
        .get_mut(&component_key)
        .ok_or_else(|| error("component does not exist".to_owned()))?;
    let segments = operation.target.path.segments();
    match &operation.action {
        OverrideAction::Set(value) => {
            let (parent, field) =
                parent_and_field(component.value_mut(), segments).map_err(error)?;
            match parent.reflect_mut() {
                ReflectMut::Struct(parent) => {
                    if let Some(existing) = parent.field_mut(&field) {
                        existing
                            .try_apply(value.value())
                            .map_err(|apply| error(apply.to_string()))?;
                    } else if let Some(parent) = parent
                        .as_partial_reflect_mut()
                        .try_downcast_mut::<DynamicStruct>()
                    {
                        parent.insert_boxed(field.clone(), clone_partial(value.value()));
                    } else {
                        return Err(error(format!("field `{field}` is absent")));
                    }
                }
                ReflectMut::Enum(parent) => {
                    let existing = parent
                        .field_mut(&field)
                        .ok_or_else(|| error(format!("enum field `{field}` is absent")))?;
                    existing
                        .try_apply(value.value())
                        .map_err(|apply| error(apply.to_string()))?;
                }
                _ => {
                    return Err(error(
                        "parent path is not a named struct or enum".to_owned(),
                    ));
                }
            }
            Ok(())
        }
        OverrideAction::Clear => {
            let (parent, field) =
                parent_and_field(component.value_mut(), segments).map_err(error)?;
            let Some(parent) = parent.try_downcast_mut::<DynamicStruct>() else {
                return Err(error("clear requires a sparse named struct".to_owned()));
            };
            if parent.remove_by_name(&field).is_none() {
                return Err(error(format!("field `{field}` is not explicitly authored")));
            }
            Ok(())
        }
        action => {
            let target = value_at_path_mut(component.value_mut(), segments).map_err(error)?;
            apply_collection_action(target, action).map_err(error)
        }
    }
}

fn parent_and_field<'a>(
    root: &'a mut dyn PartialReflect,
    segments: &[String],
) -> Result<(&'a mut dyn PartialReflect, String), String> {
    let (field, parent) = segments
        .split_last()
        .ok_or_else(|| "override path is empty".to_owned())?;
    Ok((value_at_path_mut(root, parent)?, field.clone()))
}

fn value_at_path_mut<'a>(
    mut value: &'a mut dyn PartialReflect,
    segments: &[String],
) -> Result<&'a mut dyn PartialReflect, String> {
    for segment in segments {
        value = match value.reflect_mut() {
            ReflectMut::Struct(value) => value
                .field_mut(segment)
                .ok_or_else(|| format!("field `{segment}` is absent"))?,
            ReflectMut::Enum(value) => value
                .field_mut(segment)
                .ok_or_else(|| format!("enum field `{segment}` is absent"))?,
            _ => return Err(format!("cannot traverse named field `{segment}`")),
        };
    }
    Ok(value)
}

fn apply_collection_action(
    target: &mut dyn PartialReflect,
    action: &OverrideAction,
) -> Result<(), String> {
    let ReflectMut::List(list) = target.reflect_mut() else {
        return Err("collection action target is not a list".to_owned());
    };
    match action {
        OverrideAction::Insert { index, value } => {
            if *index > list.len() {
                return Err(format!(
                    "insert index {index} exceeds length {}",
                    list.len()
                ));
            }
            list.insert(*index, clone_partial(value.value()));
        }
        OverrideAction::Remove { index } => {
            if *index >= list.len() {
                return Err(format!(
                    "remove index {index} exceeds length {}",
                    list.len()
                ));
            }
            list.remove(*index);
        }
        OverrideAction::Move { from, to } => {
            if *from >= list.len() || *to >= list.len() {
                return Err(format!("move {from}->{to} exceeds length {}", list.len()));
            }
            let value = list.remove(*from);
            list.insert(*to, value);
        }
        OverrideAction::Set(_) | OverrideAction::Clear => {
            return Err("non-collection action reached collection handler".to_owned());
        }
    }
    Ok(())
}

fn clone_partial(value: &dyn PartialReflect) -> Box<dyn PartialReflect> {
    value
        .reflect_clone()
        .map_or_else(|_| value.to_dynamic(), PartialReflect::into_partial_reflect)
}

fn validate_resolved_hierarchy(
    source_path: &str,
    entities: &ResolvedEntities,
) -> Result<(), PrefabAzSceneProcessError> {
    for (key, entity) in entities {
        if let Some(parent) = &entity.parent
            && !entities.contains_key(parent)
        {
            return Err(PrefabAzSceneProcessError::MissingParent {
                source_path: source_path.to_owned(),
                entity_alias: key.source_alias(),
                parent_alias: parent.source_alias(),
            });
        }
        let mut cursor = Some(key);
        let mut seen = BTreeSet::new();
        while let Some(next) = cursor {
            if !seen.insert(next) {
                return Err(PrefabAzSceneProcessError::HierarchyCycle {
                    source_path: source_path.to_owned(),
                    entity_alias: next.source_alias(),
                });
            }
            cursor = entities.get(next).and_then(|entity| entity.parent.as_ref());
        }
    }
    Ok(())
}

struct ConstructedPrefabWorld {
    world: World,
    entity_order: Vec<bevy::ecs::entity::Entity>,
    entity_metadata: Vec<AzSceneEntityMetadata>,
    source_scopes: Vec<AzSceneSourceScopeMetadata>,
    source_only_component_types: BTreeMap<bevy::ecs::entity::Entity, BTreeSet<TypeId>>,
}

/// Stable identity layout for one scene: local entity ids, the source-scope ids
/// every entity is filed under, and those scopes' parent metadata.
struct SceneIdentityLayout {
    local_ids: BTreeMap<EntityKey, LocalEntityId>,
    source_scope_ids: BTreeMap<Vec<String>, LocalEntityScopeId>,
    source_scopes: Vec<AzSceneSourceScopeMetadata>,
}

/// Assigns the scene's local entity and source-scope ids before any world is
/// built, and rejects two entities that claim one authored entity id.
fn plan_scene_identity(
    source_path: &str,
    entities: &ResolvedEntities,
) -> Result<SceneIdentityLayout, PrefabAzSceneProcessError> {
    if u32::try_from(entities.len()).is_err() {
        return Err(PrefabAzSceneProcessError::TooManyEntities {
            source_path: source_path.to_owned(),
            count: entities.len(),
        });
    }
    let local_ids = entities
        .keys()
        .cloned()
        .zip(0_u32..)
        .map(|(key, index)| (key, LocalEntityId::new(index)))
        .collect::<BTreeMap<_, _>>();
    let mut source_chain_set = BTreeSet::from([Vec::new()]);
    source_chain_set.extend(entities.keys().flat_map(|key| {
        (0..=key.instance_chain.len()).map(|length| key.instance_chain[..length].to_vec())
    }));
    let mut source_chains = source_chain_set.into_iter().collect::<Vec<_>>();
    source_chains.sort_by(|left, right| left.len().cmp(&right.len()).then_with(|| left.cmp(right)));
    let source_scope_ids = source_chains
        .iter()
        .cloned()
        .zip(0_u32..)
        .map(|(chain, index)| (chain, LocalEntityScopeId::new(index)))
        .collect::<BTreeMap<_, _>>();
    let source_scopes = source_chains
        .iter()
        .map(|chain| AzSceneSourceScopeMetadata {
            parent: (!chain.is_empty()).then(|| source_scope_ids[&chain[..chain.len() - 1]]),
        })
        .collect::<Vec<_>>();
    let mut authored_entity_ids = BTreeMap::new();
    for (key, resolved) in entities {
        let Some(source_entity_id) = resolved.source_entity_id.filter(|id| id.is_valid()) else {
            continue;
        };
        let source_scope = source_scope_ids[&key.instance_chain];
        if let Some(first_entity_alias) =
            authored_entity_ids.insert((source_scope, source_entity_id.value()), key.source_alias())
        {
            return Err(PrefabAzSceneProcessError::DuplicateAuthoredEntityId {
                source_path: source_path.to_owned(),
                entity_id: source_entity_id,
                first_entity_alias,
                duplicate_entity_alias: key.source_alias(),
            });
        }
    }

    Ok(SceneIdentityLayout {
        local_ids,
        source_scope_ids,
        source_scopes,
    })
}

/// Per-entity component construction: builds each component into the world,
/// runs its validation callback, and records runtime lowering targets.
/// The resolved prefab and the registries component construction reads.
///
/// Read-only for the whole pass, and none of it varies per entity.
struct ComponentConstructionInputs<'a> {
    source_path: &'a str,
    entities: &'a ResolvedEntities,
    entity_map: &'a BTreeMap<EntityKey, bevy::ecs::entity::Entity>,
    prefab_registry: &'a PrefabRegistry<'a>,
    registry: &'a TypeRegistry,
    lowerings: &'a BTreeMap<TypeId, &'a ComponentLoweringRegistration>,
}

/// The scene state component construction accumulates into.
///
/// Every field is written as components are built; they are grouped because
/// they are the one growing scene, not because they share a type.
struct SceneUnderConstruction<'a> {
    world: &'a mut World,
    references: &'a mut SceneEntityReferences,
    component_targets: &'a mut BTreeMap<bevy::ecs::entity::Entity, Vec<AzSceneComponentTarget>>,
    source_only_component_types: &'a mut BTreeMap<bevy::ecs::entity::Entity, BTreeSet<TypeId>>,
}

fn construct_entity_components(
    inputs: &ComponentConstructionInputs<'_>,
    scene: &mut SceneUnderConstruction<'_>,
) -> Result<(), PrefabAzSceneProcessError> {
    for (key, resolved) in inputs.entities {
        let entity = inputs.entity_map[key];
        for (tag, sparse) in &resolved.components {
            construct_one_component(inputs, scene, key, entity, tag, sparse)?;
        }
    }
    Ok(())
}

/// Build, validate and insert one component onto one already-spawned entity.
fn construct_one_component(
    inputs: &ComponentConstructionInputs<'_>,
    scene: &mut SceneUnderConstruction<'_>,
    key: &EntityKey,
    entity: bevy::ecs::entity::Entity,
    tag: &str,
    sparse: &SparseValue,
) -> Result<(), PrefabAzSceneProcessError> {
    let source_path = inputs.source_path;
    let prefab_type = inputs.prefab_registry.resolve_tag(tag)?;
    let registration = prefab_type.registration;
    let type_path = registration.type_info().type_path();
    let source_value = sparse.clone();
    let prefab = registration.data::<PrefabTypeData>().ok_or_else(|| {
        PrefabAzSceneProcessError::MissingPrefabTypeData {
            source_path: source_path.to_owned(),
            entity_alias: key.source_alias(),
            type_path: type_path.to_owned(),
        }
    })?;
    let built = {
        let mut entity = scene.world.entity_mut(entity);
        (prefab.construct)(
            registration,
            source_value.value(),
            &mut entity,
            scene.references,
        )
        .map_err(|source| PrefabAzSceneProcessError::ConstructComponent {
            source_path: source_path.to_owned(),
            entity_alias: key.source_alias(),
            type_path: type_path.to_owned(),
            source,
        })?
    };
    if prefab.product_policy == PrefabProductPolicy::SourceOnly {
        scene
            .source_only_component_types
            .entry(entity)
            .or_default()
            .insert(built.type_info.type_id());
    }
    if let Some(validation) = registration.data::<ValidationTypeData>() {
        let diagnostics = (validation.validate)(built.value.as_ref()).map_err(|source| {
            PrefabAzSceneProcessError::ValidationCallback {
                source_path: source_path.to_owned(),
                entity_alias: key.source_alias(),
                type_path: type_path.to_owned(),
                message: source.to_string(),
            }
        })?;
        if let Some(diagnostic) = diagnostics
            .into_iter()
            .find(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        {
            return Err(PrefabAzSceneProcessError::ComponentValidation {
                source_path: source_path.to_owned(),
                entity_alias: key.source_alias(),
                type_path: type_path.to_owned(),
                reflected_path: format!("{:?}", diagnostic.path),
                message: diagnostic.message,
            });
        }
    }
    {
        let mut entity = scene.world.entity_mut(entity);
        (prefab.insert)(
            registration,
            built.value.as_ref(),
            &mut entity,
            inputs.registry,
        )
        .map_err(|source| PrefabAzSceneProcessError::ConstructComponent {
            source_path: source_path.to_owned(),
            entity_alias: key.source_alias(),
            type_path: type_path.to_owned(),
            source,
        })?;
    }
    if prefab.product_policy == PrefabProductPolicy::Runtime
        && let Some(lowering) = inputs.lowerings.get(&registration.type_id()).copied()
        && let Some(component_id) = lowering
            .bevy_component
            .and_then(|component| component.component_id)
            .and_then(|extract| extract(&*scene.world, entity))
    {
        scene
            .component_targets
            .entry(entity)
            .or_default()
            .push(AzSceneComponentTarget {
                native_type_id: lowering.type_registration.native_type_id,
                component_id,
            });
    }
    Ok(())
}

fn construct_world(
    source_path: &str,
    registry: &TypeRegistry,
    app_registry: &AppTypeRegistry,
    lowerings: &[ComponentLoweringRegistration],
    entities: &ResolvedEntities,
) -> Result<ConstructedPrefabWorld, PrefabAzSceneProcessError> {
    let prefab_registry = PrefabRegistry::try_new(registry)?;
    let SceneIdentityLayout {
        local_ids,
        source_scope_ids,
        source_scopes,
    } = plan_scene_identity(source_path, entities)?;

    let mut world = World::new();
    world.insert_resource(app_registry.clone());
    let entity_map = entities
        .keys()
        .cloned()
        .map(|key| {
            let entity = world.spawn_empty().id();
            (key, entity)
        })
        .collect::<BTreeMap<_, _>>();
    let mut references = SceneEntityReferences::default();
    let lowerings = lowerings
        .iter()
        .map(|lowering| ((lowering.type_registration.rust_type_id)(), lowering))
        .collect::<BTreeMap<_, _>>();
    let mut component_targets =
        BTreeMap::<bevy::ecs::entity::Entity, Vec<AzSceneComponentTarget>>::new();
    let mut source_only_component_types =
        BTreeMap::<bevy::ecs::entity::Entity, BTreeSet<TypeId>>::new();

    construct_entity_components(
        &ComponentConstructionInputs {
            source_path,
            entities,
            entity_map: &entity_map,
            prefab_registry: &prefab_registry,
            registry,
            lowerings: &lowerings,
        },
        &mut SceneUnderConstruction {
            world: &mut world,
            references: &mut references,
            component_targets: &mut component_targets,
            source_only_component_types: &mut source_only_component_types,
        },
    )?;

    for (key, resolved) in entities {
        if let Some(parent) = &resolved.parent {
            world
                .entity_mut(entity_map[key])
                .insert(ChildOf(entity_map[parent]));
        }
    }
    let entity_order = entities.keys().map(|key| entity_map[key]).collect();
    let metadata = entities
        .iter()
        .map(|(key, entity)| {
            let mut targets = component_targets
                .remove(&entity_map[key])
                .unwrap_or_default();
            targets.sort_unstable_by_key(|target| target.native_type_id);
            AzSceneEntityMetadata {
                source_alias: key.source_alias(),
                source_scope: source_scope_ids[&key.instance_chain],
                source_entity_id: entity.source_entity_id.filter(|id| id.is_valid()),
                parent: entity.parent.as_ref().map(|parent| local_ids[parent]),
                component_targets: targets,
            }
        })
        .collect();
    Ok(ConstructedPrefabWorld {
        world,
        entity_order,
        entity_metadata: metadata,
        source_scopes,
        source_only_component_types,
    })
}

fn escape_alias_segment(value: &str) -> String {
    value.replace('%', "%25").replace('/', "%2F")
}

/// The six identifying fields of a failed override target.
///
/// Boxed at [`PrefabAzSceneProcessError::OverrideTarget`]: six `String`s inline
/// made that one variant dominate the enum's size, so every `Result` in this
/// module paid for it.
#[derive(Debug)]
pub struct OverrideTargetFailure {
    pub source_path: String,
    pub instance_alias: String,
    pub entity_alias: String,
    pub component: String,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum PrefabAzSceneProcessError {
    #[error("failed to initialize typed Prefab registry: {0}")]
    Registry(#[from] az_prefab::PrefabMigrationError),
    #[error("Prefab source `{source_path}` failed decode/migration/validation: {source}")]
    Source {
        source_path: String,
        #[source]
        source: PrefabCodecError,
    },
    #[error("Prefab source `{source_path}` failed typed validation: {source}")]
    TypedValidation {
        source_path: String,
        #[source]
        source: TypedPrefabSemanticsError,
    },
    #[error(
        "Prefab source `{source_path}` instance `{instance_alias}` failed to resolve `{nested_path}`: {message}"
    )]
    ResolveSource {
        source_path: String,
        instance_alias: String,
        nested_path: String,
        message: String,
    },
    #[error("nested Prefab source cycle: {chain:?}")]
    SourceCycle { chain: Vec<String> },
    #[error(
        "Prefab source `{source_path}` entity/instance `{entity_alias}` references missing parent `{parent_alias}`"
    )]
    MissingParent {
        source_path: String,
        entity_alias: String,
        parent_alias: String,
    },
    #[error("Prefab source `{source_path}` hierarchy cycles through `{entity_alias}`")]
    HierarchyCycle {
        source_path: String,
        entity_alias: String,
    },
    #[error("Prefab source `{source_path}` flattened duplicate alias `{alias}`")]
    DuplicateFlattenedAlias { source_path: String, alias: String },
    #[error(
        "Prefab source `{source_path}` has {count} entities, exceeding AZSCENE local identity capacity"
    )]
    TooManyEntities { source_path: String, count: usize },
    #[error(
        "Prefab source `{source_path}` assigns authored entity id {entity_id:?} to both `{first_entity_alias}` and `{duplicate_entity_alias}`"
    )]
    DuplicateAuthoredEntityId {
        source_path: String,
        entity_id: EntityId,
        first_entity_alias: String,
        duplicate_entity_alias: String,
    },
    #[error(
        "Prefab source `{}` instance `{}` override `{}` `{}` `{}` failed: {}",
        .0.source_path,
        .0.instance_alias,
        .0.entity_alias,
        .0.component,
        .0.path,
        .0.message
    )]
    OverrideTarget(Box<OverrideTargetFailure>),
    #[error(
        "Prefab source `{source_path}` entity `{entity_alias}` type `{type_path}` is missing PrefabTypeData"
    )]
    MissingPrefabTypeData {
        source_path: String,
        entity_alias: String,
        type_path: String,
    },
    #[error(
        "Prefab source `{source_path}` entity `{entity_alias}` type `{type_path}` construction failed: {source}"
    )]
    ConstructComponent {
        source_path: String,
        entity_alias: String,
        type_path: String,
        #[source]
        source: PrefabBuildError,
    },
    #[error(
        "Prefab source `{source_path}` entity `{entity_alias}` type `{type_path}` validation callback failed: {message}"
    )]
    ValidationCallback {
        source_path: String,
        entity_alias: String,
        type_path: String,
        message: String,
    },
    #[error(
        "Prefab source `{source_path}` entity `{entity_alias}` type `{type_path}` path `{reflected_path}` failed validation: {message}"
    )]
    ComponentValidation {
        source_path: String,
        entity_alias: String,
        type_path: String,
        reflected_path: String,
        message: String,
    },
    #[error("failed to encode AZSCENE v1: {0}")]
    Encode(#[from] AzSceneCodecError),
    #[error("failed to establish canonical AZSCENE entity order: {0}")]
    EntityOrder(#[from] AzSceneEntityOrderError),
    #[error("failed to initialize source codec: {0}")]
    Codec(#[from] PrefabCodecError),
}
