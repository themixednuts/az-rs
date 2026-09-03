//! Registered world materialization and exact instance teardown.

use std::collections::BTreeSet;

use az_core::EntityId;
use bevy::{
    ecs::{
        entity::{Entity, EntityHashMap},
        hierarchy::ChildOf,
        world::World,
    },
    reflect::TypeRegistry,
    world_serialization::WorldInstanceSpawnError,
};
use thiserror::Error;
use tracing::instrument;

use crate::{AzSceneAsset, LocalEntityId};

/// One isolated materialized scene instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AzSceneInstance {
    /// Every destination entity in product-local order.
    pub entities: Vec<Entity>,
    /// Destination entities whose product records have no parent.
    pub roots: Vec<Entity>,
    removal_order: Vec<Entity>,
}

impl AzSceneInstance {
    /// Despawns only this instance, deepest children first. Calling this again
    /// is harmless and returns zero.
    #[instrument(skip_all, fields(entity_count = self.entities.len()))]
    pub fn remove(&mut self, world: &mut World) -> usize {
        let mut removed = 0;
        for entity in self.removal_order.drain(..) {
            if let Ok(entity) = world.get_entity_mut(entity) {
                entity.despawn();
                removed += 1;
            }
        }
        self.entities.clear();
        self.roots.clear();
        removed
    }
}

impl AzSceneAsset {
    /// Materializes registered components into a fresh, isolated entity map.
    ///
    /// # Errors
    ///
    /// Returns an error before allocation for malformed hierarchy metadata, or
    /// rolls back all newly allocated entities when reflected materialization
    /// fails.
    #[instrument(skip_all, fields(entity_count = self.dynamic_world.entities.len()))]
    pub fn materialize(
        &self,
        world: &mut World,
        registry: &TypeRegistry,
    ) -> Result<AzSceneInstance, AzSceneMaterializeError> {
        let depths = hierarchy_depths(self)?;
        let mut entity_map = EntityHashMap::default();
        let mut entities = Vec::with_capacity(self.dynamic_world.entities.len());
        for dynamic in &self.dynamic_world.entities {
            let destination = world.spawn_empty().id();
            entity_map.insert(dynamic.entity, destination);
            entities.push(destination);
        }

        if let Err(source) =
            self.dynamic_world
                .write_to_world_with(world, &mut entity_map, registry)
        {
            for entity in entities.iter().rev() {
                world.despawn(*entity);
            }
            return Err(AzSceneMaterializeError::Write { source });
        }

        for (index, metadata) in self.metadata.entities.iter().enumerate() {
            let Some(parent) = metadata.parent else {
                continue;
            };
            let child = entities[index];
            let parent = entities[usize::try_from(parent.value()).map_err(|_| {
                AzSceneMaterializeError::InvalidParent {
                    entity: u32::try_from(index).unwrap_or(u32::MAX),
                    parent: parent.value(),
                    entity_count: u32::try_from(entities.len()).unwrap_or(u32::MAX),
                }
            })?];
            // The product hierarchy metadata is authoritative. Dropping a
            // reflected relationship first guarantees Bevy's relationship
            // hooks rebuild `Children` exactly once for the destination map.
            world.entity_mut(child).remove::<ChildOf>();
            world.entity_mut(child).insert(ChildOf(parent));
        }

        let roots = self
            .metadata
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, metadata)| metadata.parent.is_none().then_some(entities[index]))
            .collect();
        let mut order = (0..entities.len()).collect::<Vec<_>>();
        order.sort_by_key(|index| (std::cmp::Reverse(depths[*index]), std::cmp::Reverse(*index)));
        let removal_order = order.into_iter().map(|index| entities[index]).collect();

        Ok(AzSceneInstance {
            entities,
            roots,
            removal_order,
        })
    }
}

fn validate_source_scopes(asset: &AzSceneAsset) -> Result<(), AzSceneMaterializeError> {
    if asset.metadata.source_scopes.is_empty() {
        return Err(AzSceneMaterializeError::MissingRootSourceScope);
    }
    for (index, scope) in asset.metadata.source_scopes.iter().enumerate() {
        let source_scope = u32::try_from(index).unwrap_or(u32::MAX);
        match (source_scope, scope.parent) {
            (0, None) => {}
            (0, Some(parent)) => {
                return Err(AzSceneMaterializeError::InvalidSourceScopeParent {
                    source_scope,
                    parent: parent.value(),
                });
            }
            (_, Some(parent)) if parent.value() < source_scope => {}
            (_, Some(parent)) => {
                return Err(AzSceneMaterializeError::InvalidSourceScopeParent {
                    source_scope,
                    parent: parent.value(),
                });
            }
            (_, None) => {
                return Err(AzSceneMaterializeError::MissingSourceScopeParent { source_scope });
            }
        }
    }
    Ok(())
}

fn hierarchy_depths(asset: &AzSceneAsset) -> Result<Vec<usize>, AzSceneMaterializeError> {
    if asset.metadata.entities.len() != asset.dynamic_world.entities.len() {
        return Err(AzSceneMaterializeError::MetadataEntityCount {
            metadata: asset.metadata.entities.len(),
            entities: asset.dynamic_world.entities.len(),
        });
    }
    let entity_count = u32::try_from(asset.metadata.entities.len()).unwrap_or(u32::MAX);
    validate_source_scopes(asset)?;
    let source_scope_count = u32::try_from(asset.metadata.source_scopes.len()).unwrap_or(u32::MAX);
    let mut source_entity_ids = BTreeSet::new();
    for (index, metadata) in asset.metadata.entities.iter().enumerate() {
        let entity = u32::try_from(index).unwrap_or(u32::MAX);
        if metadata.source_scope.value() >= source_scope_count {
            return Err(AzSceneMaterializeError::InvalidEntitySourceScope {
                entity,
                source_scope: metadata.source_scope.value(),
                source_scope_count,
            });
        }
        let Some(source_entity_id) = metadata.source_entity_id else {
            continue;
        };
        if source_entity_id.is_invalid() {
            return Err(AzSceneMaterializeError::InvalidSourceEntityId { entity });
        }
        if !source_entity_ids.insert((metadata.source_scope, source_entity_id.value())) {
            return Err(AzSceneMaterializeError::DuplicateSourceEntityId {
                entity,
                source_entity_id,
            });
        }
    }
    let mut depths = Vec::with_capacity(asset.metadata.entities.len());
    for start in 0..asset.metadata.entities.len() {
        let mut depth = 0;
        let mut seen = BTreeSet::new();
        let mut cursor = Some(LocalEntityId::new(u32::try_from(start).unwrap_or(u32::MAX)));
        while let Some(local) = cursor {
            if local.value() >= entity_count {
                return Err(AzSceneMaterializeError::InvalidParent {
                    entity: u32::try_from(start).unwrap_or(u32::MAX),
                    parent: local.value(),
                    entity_count,
                });
            }
            if !seen.insert(local) {
                return Err(AzSceneMaterializeError::HierarchyCycle {
                    entity: local.value(),
                });
            }
            cursor = asset.metadata.entities[usize::try_from(local.value()).unwrap_or(usize::MAX)]
                .parent;
            if cursor.is_some() {
                depth += 1;
            }
        }
        depths.push(depth);
    }
    Ok(depths)
}

#[derive(Debug, Error)]
pub enum AzSceneMaterializeError {
    #[error("AZSCENE metadata has {metadata} entities but the dynamic world has {entities}")]
    MetadataEntityCount { metadata: usize, entities: usize },
    #[error("AZSCENE entity {entity} has invalid parent {parent} for {entity_count} entities")]
    InvalidParent {
        entity: u32,
        parent: u32,
        entity_count: u32,
    },
    #[error("AZSCENE hierarchy contains a cycle through entity {entity}")]
    HierarchyCycle { entity: u32 },
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
    #[error("AZSCENE entity {entity} contains an invalid authored source entity id")]
    InvalidSourceEntityId { entity: u32 },
    #[error("AZSCENE entity {entity} duplicates authored source entity id {source_entity_id:?}")]
    DuplicateSourceEntityId {
        entity: u32,
        source_entity_id: EntityId,
    },
    #[error("failed to materialize AZSCENE reflected world: {source}")]
    Write {
        #[source]
        source: WorldInstanceSpawnError,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removal_ignores_entities_already_despawned_by_hierarchy() {
        let mut world = World::new();
        let root = world.spawn_empty().id();
        let child = world.spawn(ChildOf(root)).id();
        let mut instance = AzSceneInstance {
            entities: vec![root, child],
            roots: vec![root],
            removal_order: vec![child, root],
        };

        world.entity_mut(root).despawn();

        assert!(world.get_entity(root).is_err());
        assert!(world.get_entity(child).is_err());
        assert_eq!(instance.remove(&mut world), 0);
        assert!(instance.entities.is_empty());
        assert!(instance.roots.is_empty());
        assert_eq!(instance.remove(&mut world), 0);
    }
}
