use std::collections::HashMap;

use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::{
    entity::Entity,
    lifecycle::RemovedComponents,
    prelude::{Changed, Query, ResMut, Resource},
};

use super::EntityId;

/// Bidirectional lookup between stable AZ entity ids and live ECS entities.
#[derive(Resource, Debug, Default)]
pub struct AzEntityIndex {
    by_id: HashMap<EntityId, Entity>,
    by_entity: HashMap<Entity, EntityId>,
}

impl AzEntityIndex {
    #[inline]
    #[must_use]
    pub fn resolve(&self, id: EntityId) -> Option<Entity> {
        self.by_id.get(&id).copied()
    }

    #[inline]
    #[must_use]
    pub fn entity_id(&self, entity: Entity) -> Option<EntityId> {
        self.by_entity.get(&entity).copied()
    }

    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    #[must_use]
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (EntityId, Entity)> + '_ {
        self.by_id.iter().map(|(&id, &entity)| (id, entity))
    }

    fn insert(&mut self, entity: Entity, id: EntityId) {
        if !id.is_valid() {
            self.remove_entity(entity);
            return;
        }

        if let Some(previous_id) = self.by_entity.insert(entity, id)
            && previous_id != id
        {
            self.by_id.remove(&previous_id);
        }
        if let Some(previous_entity) = self.by_id.insert(id, entity)
            && previous_entity != entity
        {
            self.by_entity.remove(&previous_entity);
        }
    }

    fn remove_entity(&mut self, entity: Entity) {
        if let Some(id) = self.by_entity.remove(&entity)
            && self.by_id.get(&id) == Some(&entity)
        {
            self.by_id.remove(&id);
        }
    }
}

/// Maintains [`AzEntityIndex`] from the lifecycle of [`EntityId`] components.
pub struct AzEntityIndexPlugin;

impl Plugin for AzEntityIndexPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AzEntityIndex>()
            .add_systems(PreUpdate, maintain_az_entity_index);
    }
}

fn maintain_az_entity_index(
    mut index: ResMut<AzEntityIndex>,
    changed: Query<(Entity, &EntityId), Changed<EntityId>>,
    mut removed: RemovedComponents<EntityId>,
) {
    for entity in removed.read() {
        index.remove_entity(entity);
    }
    for (entity, &id) in &changed {
        index.insert(entity, id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_tracks_changes_removals_and_invalid_ids() {
        let mut app = App::new();
        app.add_plugins(AzEntityIndexPlugin);
        let entity = app.world_mut().spawn(EntityId::new(7)).id();
        app.update();

        assert_eq!(
            app.world()
                .resource::<AzEntityIndex>()
                .resolve(EntityId::new(7)),
            Some(entity)
        );

        app.world_mut().entity_mut(entity).insert(EntityId::new(9));
        app.update();
        let index = app.world().resource::<AzEntityIndex>();
        assert_eq!(index.resolve(EntityId::new(7)), None);
        assert_eq!(index.resolve(EntityId::new(9)), Some(entity));

        app.world_mut().entity_mut(entity).insert(EntityId::INVALID);
        app.update();
        assert!(app.world().resource::<AzEntityIndex>().is_empty());
    }
}
