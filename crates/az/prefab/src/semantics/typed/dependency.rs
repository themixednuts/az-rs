use std::collections::BTreeSet;

use crate::{PrefabAssetPath, PrefabDocument};

use super::{TypedPrefabSemanticsError, TypedPrefabSourceResolver};

pub(super) fn dependency_order(
    source: &PrefabAssetPath,
    document: &PrefabDocument,
    resolver: &impl TypedPrefabSourceResolver,
) -> Result<Vec<PrefabAssetPath>, TypedPrefabSemanticsError> {
    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    let mut ordered = Vec::new();
    visit(
        source,
        document,
        resolver,
        &mut visiting,
        &mut visited,
        &mut ordered,
    )?;
    debug_assert_eq!(ordered.last(), Some(source));
    ordered.pop();
    Ok(ordered)
}

fn visit(
    source: &PrefabAssetPath,
    document: &PrefabDocument,
    resolver: &impl TypedPrefabSourceResolver,
    visiting: &mut Vec<PrefabAssetPath>,
    visited: &mut BTreeSet<PrefabAssetPath>,
    ordered: &mut Vec<PrefabAssetPath>,
) -> Result<(), TypedPrefabSemanticsError> {
    if visited.contains(source) {
        return Ok(());
    }
    if let Some(start) = visiting.iter().position(|candidate| candidate == source) {
        let mut sources = visiting[start..]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        sources.push(source.to_string());
        return Err(TypedPrefabSemanticsError::DependencyCycle { sources });
    }

    visiting.push(source.clone());
    for dependency in document.direct_dependencies() {
        let nested = resolver
            .resolve(dependency)
            .ok_or_else(|| TypedPrefabSemanticsError::MissingDependency(dependency.to_string()))?;
        visit(dependency, nested, resolver, visiting, visited, ordered)?;
    }
    visiting.pop();
    if visited.insert(source.clone()) {
        ordered.push(source.clone());
    }
    Ok(())
}
