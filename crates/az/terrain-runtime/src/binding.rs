//! Bevy binding for native terrain products.

use std::collections::HashSet;

use bevy_app::{App, Plugin, PreUpdate};
use bevy_asset::{AssetApp, AssetEvent, AssetId, AssetServer, Assets, Handle};
use bevy_ecs::prelude::*;
use bevy_reflect::Reflect;
use thiserror::Error;

use crate::{
    TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION, TERRAIN_REGION_PRODUCT_EXTENSION,
    TERRAIN_WORLD_PRODUCT_EXTENSION, TerrainHeightSource, TerrainHeightmapAsset,
    TerrainHeightmapAssetLoader, TerrainLayerSetAsset, TerrainLayerSetAssetLoader,
    TerrainRegionAsset, TerrainRegionAssetLoader, TerrainRegionRef, TerrainWorldAsset,
    TerrainWorldAssetLoader,
};

/// Authoritative native terrain-world product selected for one runtime world.
#[derive(Component, Reflect, Debug, Clone, Default, PartialEq, Eq)]
#[reflect(Component)]
pub struct TerrainWorldReference {
    asset_path: String,
}

impl TerrainWorldReference {
    #[must_use]
    pub fn new(asset_path: impl Into<String>) -> Self {
        Self {
            asset_path: asset_path.into(),
        }
    }

    #[must_use]
    pub fn asset_path(&self) -> &str {
        &self.asset_path
    }
}

impl AsRef<str> for TerrainWorldReference {
    fn as_ref(&self) -> &str {
        self.asset_path()
    }
}

impl From<String> for TerrainWorldReference {
    fn from(asset_path: String) -> Self {
        Self::new(asset_path)
    }
}

impl From<&str> for TerrainWorldReference {
    fn from(asset_path: &str) -> Self {
        Self::new(asset_path)
    }
}

/// Loaded world product associated with a [`TerrainWorldReference`].
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct TerrainWorldBinding {
    asset_path: String,
    handle: Handle<TerrainWorldAsset>,
}

impl TerrainWorldBinding {
    #[must_use]
    pub fn asset_path(&self) -> &str {
        &self.asset_path
    }

    #[must_use]
    pub const fn handle(&self) -> &Handle<TerrainWorldAsset> {
        &self.handle
    }
}

/// Region entities materialized from the current terrain-world product.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct TerrainWorldRegions {
    world_asset: AssetId<TerrainWorldAsset>,
    entities: Vec<Entity>,
}

impl TerrainWorldRegions {
    #[must_use]
    pub const fn world_asset(&self) -> AssetId<TerrainWorldAsset> {
        self.world_asset
    }

    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }
}

/// One loaded region reference owned by a terrain-world entity.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct TerrainRegionBinding {
    owner: Entity,
    world: Handle<TerrainWorldAsset>,
    region_ref: TerrainRegionRef,
    handle: Handle<TerrainRegionAsset>,
}

impl TerrainRegionBinding {
    #[must_use]
    pub const fn owner(&self) -> Entity {
        self.owner
    }

    #[must_use]
    pub const fn world(&self) -> &Handle<TerrainWorldAsset> {
        &self.world
    }

    #[must_use]
    pub const fn region_ref(&self) -> &TerrainRegionRef {
        &self.region_ref
    }

    #[must_use]
    pub const fn handle(&self) -> &Handle<TerrainRegionAsset> {
        &self.handle
    }
}

/// Resolved simulation height source for a loaded region.
#[derive(Component, Debug, Clone, PartialEq)]
pub enum TerrainHeightBinding {
    Heightmap {
        source_region: AssetId<TerrainRegionAsset>,
        asset_path: String,
        handle: Handle<TerrainHeightmapAsset>,
    },
    Constant {
        source_region: AssetId<TerrainRegionAsset>,
        value: f32,
    },
}

impl TerrainHeightBinding {
    #[must_use]
    pub const fn source_region(&self) -> AssetId<TerrainRegionAsset> {
        match self {
            Self::Heightmap { source_region, .. } | Self::Constant { source_region, .. } => {
                *source_region
            }
        }
    }
}

/// Retained binding error. Invalid products never create partial terrain.
#[derive(Component, Debug, Clone, PartialEq, Eq, Error)]
pub enum TerrainRuntimeBindingError {
    #[error("terrain world path is empty")]
    EmptyWorldPath,
    #[error("terrain world product '{0}' is not an .{TERRAIN_WORLD_PRODUCT_EXTENSION} asset")]
    UnsupportedWorldProduct(String),
    #[error("terrain region product '{0}' is not an .{TERRAIN_REGION_PRODUCT_EXTENSION} asset")]
    UnsupportedRegionProduct(String),
    #[error("terrain height product '{0}' is not an .{TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION} asset")]
    UnsupportedHeightProduct(String),
    #[error("runtime terrain height graph '{0}' has no compiled height product")]
    UncompiledHeightGraph(String),
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerrainRuntimeSet {
    References,
    Worlds,
    Regions,
}

/// Installs native terrain assets and their world → region → height bindings.
pub struct TerrainRuntimePlugin;

impl Plugin for TerrainRuntimePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<TerrainWorldReference>()
            .init_asset::<TerrainWorldAsset>()
            .init_asset::<TerrainRegionAsset>()
            .init_asset::<TerrainLayerSetAsset>()
            .init_asset::<TerrainHeightmapAsset>()
            .init_asset_loader::<TerrainWorldAssetLoader>()
            .init_asset_loader::<TerrainRegionAssetLoader>()
            .init_asset_loader::<TerrainLayerSetAssetLoader>()
            .init_asset_loader::<TerrainHeightmapAssetLoader>()
            .configure_sets(
                PreUpdate,
                (
                    TerrainRuntimeSet::References,
                    TerrainRuntimeSet::Worlds,
                    TerrainRuntimeSet::Regions,
                )
                    .chain(),
            )
            .add_systems(
                PreUpdate,
                (sync_world_references, cleanup_removed_world_references)
                    .chain()
                    .in_set(TerrainRuntimeSet::References),
            )
            .add_systems(
                PreUpdate,
                materialize_world_regions.in_set(TerrainRuntimeSet::Worlds),
            )
            .add_systems(
                PreUpdate,
                (materialize_region_heights, cleanup_orphaned_regions)
                    .chain()
                    .in_set(TerrainRuntimeSet::Regions),
            );
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn sync_world_references(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    references: Query<(
        Entity,
        Ref<TerrainWorldReference>,
        Option<&TerrainWorldBinding>,
    )>,
) {
    for (entity, reference, binding) in &references {
        if !reference.is_changed() && binding.is_some() {
            continue;
        }
        let asset_path = reference.asset_path().trim();
        let result = if asset_path.is_empty() {
            Err(TerrainRuntimeBindingError::EmptyWorldPath)
        } else if !is_product_path(asset_path, TERRAIN_WORLD_PRODUCT_EXTENSION) {
            Err(TerrainRuntimeBindingError::UnsupportedWorldProduct(
                asset_path.to_owned(),
            ))
        } else {
            Ok(TerrainWorldBinding {
                asset_path: asset_path.to_owned(),
                handle: asset_server.load(asset_path.to_owned()),
            })
        };
        match result {
            Ok(binding) => {
                commands
                    .entity(entity)
                    .insert(binding)
                    .remove::<TerrainRuntimeBindingError>();
            }
            Err(error) => {
                clear_world_regions(&mut commands, entity, None);
                commands
                    .entity(entity)
                    .insert(error)
                    .remove::<TerrainWorldBinding>();
            }
        }
    }
}

fn cleanup_removed_world_references(
    mut commands: Commands,
    mut removed: RemovedComponents<TerrainWorldReference>,
    regions: Query<&TerrainWorldRegions>,
) {
    for entity in removed.read() {
        let current = regions.get(entity).ok();
        clear_world_regions(&mut commands, entity, current);
        commands
            .entity(entity)
            .remove::<TerrainWorldBinding>()
            .remove::<TerrainRuntimeBindingError>();
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn materialize_world_regions(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    worlds: Res<Assets<TerrainWorldAsset>>,
    mut events: MessageReader<AssetEvent<TerrainWorldAsset>>,
    bindings: Query<(
        Entity,
        Ref<TerrainWorldBinding>,
        Option<&TerrainWorldRegions>,
    )>,
) {
    let changed = changed_assets(&mut events);
    for (entity, binding, current) in &bindings {
        let world_id = binding.handle.id();
        if !binding.is_changed()
            && current.is_some_and(|current| current.world_asset == world_id)
            && !changed.contains(&world_id)
        {
            continue;
        }
        let Some(world) = worlds.get(world_id) else {
            if changed.contains(&world_id) {
                clear_world_regions(&mut commands, entity, current);
            }
            continue;
        };
        let invalid = world
            .regions
            .iter()
            .map(|region| region.asset.trim())
            .find(|path| !is_product_path(path, TERRAIN_REGION_PRODUCT_EXTENSION));
        if let Some(path) = invalid {
            clear_world_regions(&mut commands, entity, current);
            commands
                .entity(entity)
                .insert(TerrainRuntimeBindingError::UnsupportedRegionProduct(
                    path.to_owned(),
                ));
            continue;
        }

        if let Some(current) = current {
            despawn_regions(&mut commands, &current.entities);
        }
        let entities = world
            .regions
            .iter()
            .cloned()
            .map(|region_ref| {
                let handle = asset_server.load(region_ref.asset.clone());
                commands
                    .spawn(TerrainRegionBinding {
                        owner: entity,
                        world: binding.handle.clone(),
                        region_ref,
                        handle,
                    })
                    .id()
            })
            .collect();
        commands
            .entity(entity)
            .insert(TerrainWorldRegions {
                world_asset: world_id,
                entities,
            })
            .remove::<TerrainRuntimeBindingError>();
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters must be taken by value; `&Res<_>` is not a `SystemParam`"
)]
fn materialize_region_heights(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    regions: Res<Assets<TerrainRegionAsset>>,
    mut events: MessageReader<AssetEvent<TerrainRegionAsset>>,
    bindings: Query<(
        Entity,
        Ref<TerrainRegionBinding>,
        Option<&TerrainHeightBinding>,
    )>,
) {
    let changed = changed_assets(&mut events);
    for (entity, binding, current) in &bindings {
        let region_id = binding.handle.id();
        if !binding.is_changed()
            && current.is_some_and(|current| current.source_region() == region_id)
            && !changed.contains(&region_id)
        {
            continue;
        }
        let Some(region) = regions.get(region_id) else {
            if changed.contains(&region_id) {
                commands.entity(entity).remove::<TerrainHeightBinding>();
            }
            continue;
        };
        let result = match &region.height {
            TerrainHeightSource::Image(image) => {
                heightmap_binding(region_id, image.image.trim(), &asset_server)
            }
            TerrainHeightSource::Tiled(tiles) => {
                heightmap_binding(region_id, tiles.asset.trim(), &asset_server)
            }
            TerrainHeightSource::Constant(constant) => Ok(TerrainHeightBinding::Constant {
                source_region: region_id,
                value: constant.value,
            }),
            TerrainHeightSource::Graph(graph) => Err(
                TerrainRuntimeBindingError::UncompiledHeightGraph(graph.graph.clone()),
            ),
        };
        match result {
            Ok(height) => {
                commands
                    .entity(entity)
                    .insert(height)
                    .remove::<TerrainRuntimeBindingError>();
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(error)
                    .remove::<TerrainHeightBinding>();
            }
        }
    }
}

fn cleanup_orphaned_regions(
    mut commands: Commands,
    regions: Query<(Entity, &TerrainRegionBinding)>,
    worlds: Query<(), With<TerrainWorldBinding>>,
) {
    for (entity, binding) in &regions {
        if !worlds.contains(binding.owner) {
            commands.entity(entity).despawn();
        }
    }
}

fn heightmap_binding(
    source_region: AssetId<TerrainRegionAsset>,
    asset_path: &str,
    asset_server: &AssetServer,
) -> Result<TerrainHeightBinding, TerrainRuntimeBindingError> {
    if !is_product_path(asset_path, TERRAIN_HEIGHTMAP_PRODUCT_EXTENSION) {
        return Err(TerrainRuntimeBindingError::UnsupportedHeightProduct(
            asset_path.to_owned(),
        ));
    }
    Ok(TerrainHeightBinding::Heightmap {
        source_region,
        asset_path: asset_path.to_owned(),
        handle: asset_server.load(asset_path.to_owned()),
    })
}

fn changed_assets<A: bevy_asset::Asset>(
    events: &mut MessageReader<AssetEvent<A>>,
) -> HashSet<AssetId<A>> {
    events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id }
            | AssetEvent::Removed { id } => Some(*id),
            AssetEvent::Unused { .. } => None,
        })
        .collect()
}

fn clear_world_regions(
    commands: &mut Commands,
    entity: Entity,
    current: Option<&TerrainWorldRegions>,
) {
    if let Some(current) = current {
        despawn_regions(commands, &current.entities);
    }
    commands.entity(entity).remove::<TerrainWorldRegions>();
}

fn despawn_regions(commands: &mut Commands, entities: &[Entity]) {
    for &entity in entities {
        commands.entity(entity).despawn();
    }
}

fn is_product_path(path: &str, extension: &str) -> bool {
    !path.is_empty()
        && path
            .to_ascii_lowercase()
            .ends_with(&format!(".{extension}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_reference_uses_borrowed_path_traits() {
        let reference = TerrainWorldReference::from("terrain/main.azterrain-world.bin");
        assert_eq!(reference.as_ref(), "terrain/main.azterrain-world.bin");
    }

    #[test]
    fn native_product_paths_are_unambiguous() {
        assert!(is_product_path(
            "terrain/main.azterrain-world.bin",
            TERRAIN_WORLD_PRODUCT_EXTENSION
        ));
        assert!(!is_product_path(
            "terrain-world/main.terrain-world.bin",
            TERRAIN_WORLD_PRODUCT_EXTENSION
        ));
    }
}
