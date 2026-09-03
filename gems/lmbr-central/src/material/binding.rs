//! Material asset binding components and render sync helpers.

use bevy::asset::{AssetEvent, AssetId};
use bevy::ecs::system::EntityCommands;
use bevy::prelude::*;

use super::definition::MaterialAsset;
use super::format::material_engine_asset_path;

/// Material asset currently bound to a rendered entity.
#[derive(Component, Debug, Clone, Reflect)]
#[reflect(Component)]
pub struct MaterialAssetBinding {
    path: String,
    handle: Handle<MaterialAsset>,
    slot: Option<u32>,
    applied: bool,
}

impl MaterialAssetBinding {
    #[must_use]
    pub fn new(path: impl Into<String>, handle: Handle<MaterialAsset>) -> Self {
        Self {
            path: path.into(),
            handle,
            slot: None,
            applied: false,
        }
    }

    #[must_use]
    pub const fn with_slot(mut self, slot: Option<u32>) -> Self {
        self.slot = slot;
        self
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub const fn handle(&self) -> &Handle<MaterialAsset> {
        &self.handle
    }

    #[must_use]
    pub const fn slot(&self) -> Option<u32> {
        self.slot
    }

    #[must_use]
    pub const fn is_applied(&self) -> bool {
        self.applied
    }

    pub const fn mark_stale(&mut self) {
        self.applied = false;
    }

    pub const fn mark_applied(&mut self) {
        self.applied = true;
    }
}

pub fn sync_material_asset_binding(
    entity_commands: &mut EntityCommands<'_>,
    asset_server: Option<&AssetServer>,
    asset_path: &str,
    binding: Option<&MaterialAssetBinding>,
) -> bool {
    let Some(engine_path) = material_asset_path_from_source(asset_path) else {
        if binding.is_some() {
            entity_commands.remove::<MaterialAssetBinding>();
            return true;
        }
        return false;
    };

    let Some(asset_server) = asset_server else {
        if binding.is_some() {
            entity_commands.remove::<MaterialAssetBinding>();
            return true;
        }
        return false;
    };

    let material_changed = binding.is_none_or(|binding| binding.path != engine_path);
    if material_changed {
        let handle: Handle<MaterialAsset> = asset_server.load(engine_path.clone());
        entity_commands.insert(MaterialAssetBinding::new(engine_path, handle));
    }

    material_changed
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "`Res` is an owned Bevy system parameter; a reference stops this satisfying `IntoSystem`"
)]
pub fn apply_material_asset_bindings(
    mut commands: Commands,
    mut material_asset_events: MessageReader<AssetEvent<MaterialAsset>>,
    asset_server: Option<Res<AssetServer>>,
    material_assets: Option<Res<Assets<MaterialAsset>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut query: Query<(Entity, &mut MaterialAssetBinding)>,
) {
    let (Some(material_assets), Some(materials)) = (material_assets, materials.as_deref_mut())
    else {
        return;
    };
    let asset_server = asset_server.as_deref();

    let changed_asset_ids = material_asset_events
        .read()
        .filter_map(material_asset_refresh_id)
        .collect::<Vec<_>>();

    for (entity, mut binding) in &mut query {
        if binding.is_applied()
            && changed_asset_ids
                .iter()
                .any(|id| *id == binding.handle.id())
        {
            binding.mark_stale();
        }
        if binding.is_applied() {
            continue;
        }
        let Some(material_asset) = material_assets.get(&binding.handle) else {
            continue;
        };

        let material = match asset_server {
            Some(asset_server) => material_asset
                .standard_material_for_slot_with_images(binding.slot, |path| {
                    asset_server.load(path.to_owned())
                }),
            None => material_asset.standard_material_for_slot(binding.slot),
        };
        commands
            .entity(entity)
            .insert(MeshMaterial3d(materials.add(material)));
        binding.mark_applied();
    }
}

#[must_use]
pub const fn material_asset_refresh_id(
    event: &AssetEvent<MaterialAsset>,
) -> Option<AssetId<MaterialAsset>> {
    match event {
        AssetEvent::Added { id }
        | AssetEvent::Modified { id }
        | AssetEvent::LoadedWithDependencies { id } => Some(*id),
        AssetEvent::Removed { .. } | AssetEvent::Unused { .. } => None,
    }
}

#[must_use]
pub fn material_asset_path_from_source(source_path: &str) -> Option<String> {
    let source_path = source_path.trim();
    if source_path.is_empty() {
        return None;
    }
    Some(material_engine_asset_path(source_path))
}

#[must_use]
pub fn material_asset_path_from_component(asset_path: &str) -> Option<String> {
    material_asset_path_from_source(asset_path)
}
