use std::collections::{BTreeMap, BTreeSet};

use bevy_reflect::TypeRegistry;

use crate::{PrefabDocument, PrefabRegistry};

use super::TypedPrefabSemanticsError;

pub(super) fn validate_local(
    document: &PrefabDocument,
    registry: &TypeRegistry,
) -> Result<(), TypedPrefabSemanticsError> {
    validate_alias_namespaces(document)?;
    validate_hierarchy(document)?;
    validate_component_identity(document, registry)
}

fn validate_alias_namespaces(document: &PrefabDocument) -> Result<(), TypedPrefabSemanticsError> {
    if let Some(alias) = document.entities.keys().find(|alias| {
        document
            .instances
            .keys()
            .any(|other| other.as_str() == alias.as_str())
    }) {
        return Err(TypedPrefabSemanticsError::AliasCollision(
            alias.as_str().to_owned(),
        ));
    }
    Ok(())
}

fn validate_hierarchy(document: &PrefabDocument) -> Result<(), TypedPrefabSemanticsError> {
    for (alias, entity) in &document.entities {
        if let Some(parent) = &entity.parent
            && !document.entities.contains_key(parent)
        {
            return Err(TypedPrefabSemanticsError::DanglingParent {
                entity: alias.to_string(),
                parent: parent.to_string(),
            });
        }
    }
    for (alias, instance) in &document.instances {
        if let Some(parent) = &instance.parent
            && !document.entities.contains_key(parent)
        {
            return Err(TypedPrefabSemanticsError::DanglingInstanceParent {
                instance: alias.to_string(),
                parent: parent.to_string(),
            });
        }
    }

    let mut states = BTreeMap::new();
    let mut stack = Vec::new();
    for alias in document.entities.keys() {
        visit_parent(alias, document, &mut states, &mut stack)?;
    }
    Ok(())
}

fn visit_parent(
    alias: &crate::EntityAlias,
    document: &PrefabDocument,
    states: &mut BTreeMap<crate::EntityAlias, VisitState>,
    stack: &mut Vec<crate::EntityAlias>,
) -> Result<(), TypedPrefabSemanticsError> {
    match states.get(alias) {
        Some(VisitState::Done) => return Ok(()),
        Some(VisitState::Visiting) => {
            let start = stack
                .iter()
                .position(|candidate| candidate == alias)
                .unwrap_or(0);
            let mut aliases = stack[start..]
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            aliases.push(alias.to_string());
            return Err(TypedPrefabSemanticsError::HierarchyCycle { aliases });
        }
        None => {}
    }
    states.insert(alias.clone(), VisitState::Visiting);
    stack.push(alias.clone());
    if let Some(parent) = document
        .entities
        .get(alias)
        .and_then(|entity| entity.parent.as_ref())
    {
        visit_parent(parent, document, states, stack)?;
    }
    stack.pop();
    states.insert(alias.clone(), VisitState::Done);
    Ok(())
}

fn validate_component_identity(
    document: &PrefabDocument,
    registry: &TypeRegistry,
) -> Result<(), TypedPrefabSemanticsError> {
    let prefab_registry = PrefabRegistry::try_new(registry)
        .map_err(|error| TypedPrefabSemanticsError::Registry(error.to_string()))?;
    let mut used = BTreeSet::new();
    for (entity_alias, entity) in &document.entities {
        for (tag, value) in &entity.components {
            let resolved = prefab_registry
                .resolve_tag(tag)
                .map_err(|error| TypedPrefabSemanticsError::Registry(error.to_string()))?;
            if resolved.prefab.tag != tag
                || document.type_versions.get(tag) != Some(&resolved.prefab.source_version)
            {
                return Err(TypedPrefabSemanticsError::MissingCanonicalTypeVersion {
                    entity: entity_alias.to_string(),
                    tag: tag.clone(),
                });
            }
            if value.type_info().type_id() != resolved.source_registration.type_id() {
                return Err(TypedPrefabSemanticsError::ComponentTypeMismatch {
                    entity: entity_alias.to_string(),
                    tag: tag.clone(),
                    registered: resolved
                        .source_registration
                        .type_info()
                        .type_path()
                        .to_owned(),
                    actual: value.type_path().to_owned(),
                });
            }
            used.insert(tag);
        }
    }
    if let Some(unused) = document
        .type_versions
        .keys()
        .find(|tag| !used.contains(tag))
    {
        return Err(TypedPrefabSemanticsError::UnusedCanonicalTypeVersion(
            unused.clone(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum VisitState {
    Visiting,
    Done,
}
