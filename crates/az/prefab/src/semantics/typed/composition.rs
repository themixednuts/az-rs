use std::collections::{BTreeMap, BTreeSet};

use bevy_reflect::{TypeInfo, TypeRegistry};

use crate::{
    EntityAlias, InstanceAlias, OverrideOperation, PrefabAssetPath, PrefabDocument, PrefabRegistry,
    ReflectedPath, document::OverrideAction,
};

use super::{TypedPrefabSemanticsError, TypedPrefabSourceResolver, action_name, target_label};

/// Fully-qualified override target after prefixing every containing instance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResolvedOverrideTarget {
    pub instance_chain: Vec<InstanceAlias>,
    pub entity: EntityAlias,
    pub component: String,
    pub path: ReflectedPath,
}

/// Winning authored override and the layer where it was declared.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedOverride {
    pub target: ResolvedOverrideTarget,
    pub action: OverrideAction,
    pub authored_layer: Vec<InstanceAlias>,
}

pub(super) fn resolve_override_precedence(
    source: &PrefabAssetPath,
    document: &PrefabDocument,
    resolver: &impl TypedPrefabSourceResolver,
    registry: &TypeRegistry,
) -> Result<Vec<ResolvedOverride>, TypedPrefabSemanticsError> {
    let prefab_registry = PrefabRegistry::try_new(registry)
        .map_err(|error| TypedPrefabSemanticsError::Registry(error.to_string()))?;
    let mut winners = BTreeMap::new();
    let mut source_stack = Vec::new();
    walk_document(
        source,
        document,
        &[],
        resolver,
        &prefab_registry,
        &mut source_stack,
        &mut winners,
    )?;
    Ok(winners.into_values().collect())
}

#[allow(clippy::too_many_arguments)]
fn walk_document(
    source: &PrefabAssetPath,
    document: &PrefabDocument,
    prefix: &[InstanceAlias],
    resolver: &impl TypedPrefabSourceResolver,
    registry: &PrefabRegistry<'_>,
    source_stack: &mut Vec<PrefabAssetPath>,
    winners: &mut BTreeMap<ResolvedOverrideTarget, ResolvedOverride>,
) -> Result<(), TypedPrefabSemanticsError> {
    if let Some(start) = source_stack
        .iter()
        .position(|candidate| candidate == source)
    {
        let mut sources = source_stack[start..]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        sources.push(source.to_string());
        return Err(TypedPrefabSemanticsError::DependencyCycle { sources });
    }
    source_stack.push(source.clone());

    for (instance_alias, instance) in &document.instances {
        let nested = resolver.resolve(&instance.source).ok_or_else(|| {
            TypedPrefabSemanticsError::MissingDependency(instance.source.to_string())
        })?;
        let mut layer = prefix.to_vec();
        layer.push(instance_alias.clone());

        // Inner authored links resolve first. Inserting this layer afterwards
        // gives the nearest outer authored value precedence.
        walk_document(
            &instance.source,
            nested,
            &layer,
            resolver,
            registry,
            source_stack,
            winners,
        )?;

        let mut layer_targets = BTreeSet::new();
        for operation in &instance.overrides {
            validate_target(nested, &instance.source, operation, resolver, registry)?;
            let mut chain = layer.clone();
            chain.extend(operation.target.instance_chain.iter().cloned());
            let target = ResolvedOverrideTarget {
                instance_chain: chain,
                entity: operation.target.entity.clone(),
                component: operation.target.component.clone(),
                path: operation.target.path.clone(),
            };
            if !layer_targets.insert(target.clone()) {
                return Err(TypedPrefabSemanticsError::SameLayerConflict {
                    target: target_label(
                        &target.instance_chain,
                        &target.entity,
                        &target.component,
                        &target.path,
                    ),
                });
            }
            winners.insert(
                target.clone(),
                ResolvedOverride {
                    target,
                    action: operation.action.clone(),
                    authored_layer: layer.clone(),
                },
            );
        }
    }
    source_stack.pop();
    Ok(())
}

fn validate_target<'a>(
    mut document: &'a PrefabDocument,
    initial_source: &PrefabAssetPath,
    operation: &OverrideOperation,
    resolver: &'a impl TypedPrefabSourceResolver,
    registry: &PrefabRegistry<'_>,
) -> Result<(), TypedPrefabSemanticsError> {
    let mut source = initial_source.clone();
    for instance_alias in &operation.target.instance_chain {
        let instance = document.instances.get(instance_alias).ok_or_else(|| {
            TypedPrefabSemanticsError::DanglingOverrideInstance {
                asset_source: source.to_string(),
                instance: instance_alias.to_string(),
            }
        })?;
        source = instance.source.clone();
        document = resolver
            .resolve(&source)
            .ok_or_else(|| TypedPrefabSemanticsError::MissingDependency(source.to_string()))?;
    }

    let entity = document
        .entities
        .get(&operation.target.entity)
        .ok_or_else(|| TypedPrefabSemanticsError::DanglingOverrideEntity {
            asset_source: source.to_string(),
            entity: operation.target.entity.to_string(),
        })?;
    let registration = registry
        .resolve_prefab_type_path(&operation.target.component)
        .map_err(|error| TypedPrefabSemanticsError::Registry(error.to_string()))?;
    let component = entity
        .components
        .get(registration.prefab.tag)
        .ok_or_else(|| TypedPrefabSemanticsError::DanglingOverrideComponent {
            asset_source: source.to_string(),
            entity: operation.target.entity.to_string(),
            component: operation.target.component.clone(),
        })?;
    debug_assert_eq!(
        component.type_info().type_id(),
        registration.source_registration.type_id(),
        "validated Prefab component must retain its direct or template-backed source type"
    );
    let terminal = resolve_named_path(component.type_info(), &operation.target.path)?;
    let value_type = match &operation.action {
        OverrideAction::Set(value) => Some((value, terminal)),
        OverrideAction::Insert { value, .. } => {
            Some((value, collection_element(terminal, operation)?))
        }
        OverrideAction::Remove { .. } | OverrideAction::Move { .. } => {
            collection_element(terminal, operation)?;
            None
        }
        OverrideAction::Clear => None,
    };
    if let Some((value, expected)) = value_type
        && value.type_info().type_id() != expected.type_id()
    {
        return Err(TypedPrefabSemanticsError::OverrideValueTypeMismatch {
            path: operation.target.path.to_string(),
            expected: expected.type_path().to_owned(),
            actual: value.type_path().to_owned(),
        });
    }
    Ok(())
}

fn collection_element(
    type_info: &'static TypeInfo,
    operation: &OverrideOperation,
) -> Result<&'static TypeInfo, TypedPrefabSemanticsError> {
    let item = match type_info {
        TypeInfo::List(info) => info.item_info(),
        TypeInfo::Array(info) => info.item_info(),
        _ => None,
    };
    item.ok_or_else(|| TypedPrefabSemanticsError::OverrideCollectionRequired {
        action: action_name(&operation.action),
        path: operation.target.path.to_string(),
    })
}

fn resolve_named_path(
    mut type_info: &'static TypeInfo,
    path: &ReflectedPath,
) -> Result<&'static TypeInfo, TypedPrefabSemanticsError> {
    let component = type_info.type_path().to_owned();
    for segment in path.segments() {
        let TypeInfo::Struct(info) = type_info else {
            return Err(TypedPrefabSemanticsError::InvalidOverridePath {
                component,
                path: path.to_string(),
                reason: format!("`{}` has no named fields", type_info.type_path()),
            });
        };
        type_info = info
            .field(segment)
            .and_then(bevy_reflect::NamedField::type_info)
            .ok_or_else(|| TypedPrefabSemanticsError::InvalidOverridePath {
                component: component.clone(),
                path: path.to_string(),
                reason: format!("unknown or dynamically typed field `{segment}`"),
            })?;
    }
    Ok(type_info)
}
