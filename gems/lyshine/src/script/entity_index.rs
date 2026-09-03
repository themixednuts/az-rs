//! [`LyShineEntityIndex`] — `UiEntityId → Bevy Entity` lookup.
//!
//! Lua `self.entityId` values and addresses passed to `UiElementBus`
//! and `UiCanvasBus` are [`UiEntityId`] values (a stable u64 parsed from
//! the canvas asset). Native code receiving an address in a bus
//! dispatcher needs to find the Bevy `Entity` carrying the
//! [`crate::LyShineUiEntity`] component with that id.
//!
//! The index is a `HashMap<UiEntityId, Entity>` Bevy resource maintained
//! by a system that watches `Added<LyShineUiEntity>` /
//! `RemovedComponents<LyShineUiEntity>`. Bus handlers borrow it via
//! [`bevy_mod_scripting::bindings::WorldGuard::with_resource`].
//!
//! Rebuild semantics: full from-scratch on `Added` / removed events.
//! Cheap because the canvas-element count is bounded by the canvas
//! and rebuilds are infrequent (canvases spawn once at level load).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::LyShineUiEntity;
use crate::canvas::UiEntityId;

/// `UiEntityId → Bevy Entity` index.
#[derive(Resource, Default, Debug)]
pub struct LyShineEntityIndex {
    by_ui_id: HashMap<UiEntityId, Entity>,
}

impl LyShineEntityIndex {
    /// Look up the Bevy `Entity` for a `UiEntityId`.
    #[must_use]
    pub fn get(&self, id: UiEntityId) -> Option<Entity> {
        self.by_ui_id.get(&id).copied()
    }

    /// Number of indexed entities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_ui_id.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_ui_id.is_empty()
    }
}

/// Plugin: install the index resource and maintain it from
/// [`LyShineUiEntity`] lifecycle events.
pub struct LyShineEntityIndexPlugin;

impl Plugin for LyShineEntityIndexPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LyShineEntityIndex>();
        app.add_systems(Update, maintain_index);
    }
}

fn maintain_index(
    mut index: ResMut<LyShineEntityIndex>,
    added: Query<(Entity, &LyShineUiEntity), Added<LyShineUiEntity>>,
    mut removed: RemovedComponents<LyShineUiEntity>,
) {
    let added_count = added.iter().count();
    if added_count > 0 {
        for (entity, ui) in &added {
            index.by_ui_id.insert(ui.entity_id, entity);
        }
    }

    // Removed: walk our map and drop any entry whose Entity matches a
    // removed one. RemovedComponents gives us Bevy entity ids of the
    // removed-from entities; the entity may already have been despawned,
    // so we don't have access to its UiEntityId — scan in reverse.
    let removed_entities: Vec<Entity> = removed.read().collect();
    if !removed_entities.is_empty() {
        index.by_ui_id.retain(|_, e| !removed_entities.contains(e));
    }
}
