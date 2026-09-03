use std::collections::HashSet;

use az_asset::AssetId as AzAssetId;
use az_framework::asset::{AssetCatalog, AssetRefLoadError};
use az_physics::PhysicsColliderSet;
use bevy::{
    asset::{AssetEvent, AssetId as BevyAssetId},
    prelude::*,
};

use crate::{ShapeAsset, ShapeAssetReference, ShapeConversionError, ToPhysicsColliders};

/// Resolved shape asset handle for a scene entity.
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct ShapeAssetBinding {
    asset_id: AzAssetId,
    handle: Handle<ShapeAsset>,
}

/// Marks the collider set generated from a particular loaded shape asset.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeAssetPhysicsBinding {
    asset_id: AzAssetId,
    handle_id: BevyAssetId<ShapeAsset>,
}

impl ShapeAssetPhysicsBinding {
    #[inline]
    #[must_use]
    pub const fn new(asset_id: AzAssetId, handle_id: BevyAssetId<ShapeAsset>) -> Self {
        Self {
            asset_id,
            handle_id,
        }
    }

    #[inline]
    #[must_use]
    pub const fn asset_id(&self) -> AzAssetId {
        self.asset_id
    }

    #[inline]
    #[must_use]
    pub const fn handle_id(&self) -> BevyAssetId<ShapeAsset> {
        self.handle_id
    }
}

/// Conversion failure retained on the entity instead of substituting fallback
/// geometry.
#[derive(Component, Debug, Clone, PartialEq)]
pub struct ShapeAssetPhysicsError(pub ShapeConversionError);

/// Failure to resolve an authored `RockNRoll` asset identity through the runtime
/// catalog.
#[derive(Component, Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShapeAssetReferenceError {
    #[error("RockNRoll asset {0} requires the runtime asset catalog")]
    MissingCatalog(AzAssetId),
    #[error(transparent)]
    Resolve(#[from] AssetRefLoadError),
}

impl ShapeAssetBinding {
    #[inline]
    #[must_use]
    pub const fn new(asset_id: AzAssetId, handle: Handle<ShapeAsset>) -> Self {
        Self { asset_id, handle }
    }

    #[inline]
    #[must_use]
    pub const fn asset_id(&self) -> AzAssetId {
        self.asset_id
    }

    #[inline]
    #[must_use]
    pub const fn handle(&self) -> &Handle<ShapeAsset> {
        &self.handle
    }
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing these would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_shape_asset_references(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    catalog: Option<Res<AssetCatalog>>,
    query: Query<(Entity, Ref<ShapeAssetReference>, Option<&ShapeAssetBinding>)>,
) {
    let Some(asset_server) = asset_server.as_deref() else {
        return;
    };

    for (entity, reference, binding) in &query {
        if !reference.is_changed() && binding.is_some() {
            continue;
        }

        if reference.is_empty() {
            commands
                .entity(entity)
                .remove::<ShapeAssetBinding>()
                .remove::<ShapeAssetReferenceError>();
            continue;
        }

        if binding.is_some_and(|binding| binding.asset_id() == reference.id()) {
            commands.entity(entity).remove::<ShapeAssetReferenceError>();
            continue;
        }

        let Some(catalog) = catalog.as_deref() else {
            commands
                .entity(entity)
                .insert(ShapeAssetReferenceError::MissingCatalog(reference.id()))
                .remove::<ShapeAssetBinding>();
            continue;
        };
        let asset_ref = AsRef::<crate::ShapeAssetRef>::as_ref(&*reference);
        let handle = match catalog.load_asset_ref(asset_ref, asset_server) {
            Ok(handle) => handle,
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(ShapeAssetReferenceError::from(error))
                    .remove::<ShapeAssetBinding>();
                continue;
            }
        };
        commands
            .entity(entity)
            .insert(ShapeAssetBinding::new(reference.id(), handle))
            .remove::<ShapeAssetReferenceError>();
    }
}

pub fn cleanup_removed_shape_asset_references(
    mut commands: Commands,
    mut removed: RemovedComponents<ShapeAssetReference>,
    bindings: Query<(), With<ShapeAssetBinding>>,
) {
    for entity in removed.read() {
        let has_binding = bindings.contains(entity);
        let mut entity_commands = commands.entity(entity);
        if has_binding {
            entity_commands.remove::<ShapeAssetBinding>();
        }
        entity_commands.remove::<ShapeAssetReferenceError>();
    }
}

// Bevy system: `Res` is an owned parameter wrapper, so borrowing it here would
// stop this function satisfying `IntoSystem` and it could not be registered.
#[allow(clippy::needless_pass_by_value)]
pub fn sync_shape_asset_physics(
    mut commands: Commands,
    assets: Option<Res<Assets<ShapeAsset>>>,
    mut asset_events: MessageReader<AssetEvent<ShapeAsset>>,
    query: Query<(
        Entity,
        Ref<ShapeAssetBinding>,
        Option<&ShapeAssetPhysicsBinding>,
    )>,
) {
    let Some(assets) = assets.as_deref() else {
        return;
    };
    let changed: HashSet<_> = asset_events
        .read()
        .filter_map(|event| match event {
            AssetEvent::Added { id }
            | AssetEvent::Modified { id }
            | AssetEvent::LoadedWithDependencies { id } => Some(*id),
            AssetEvent::Removed { .. } | AssetEvent::Unused { .. } => None,
        })
        .collect();

    for (entity, binding, materialized) in &query {
        let id = binding.handle().id();
        if !binding.is_changed()
            && materialized.is_some_and(|materialized| {
                materialized.asset_id() == binding.asset_id() && materialized.handle_id() == id
            })
            && !changed.contains(&id)
        {
            continue;
        }
        let Some(asset) = assets.get(id) else {
            continue;
        };
        match asset.to_physics_colliders() {
            Ok(colliders) => {
                commands
                    .entity(entity)
                    .insert((
                        colliders,
                        ShapeAssetPhysicsBinding::new(binding.asset_id(), id),
                    ))
                    .remove::<ShapeAssetPhysicsError>();
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(ShapeAssetPhysicsError(error))
                    .remove::<PhysicsColliderSet>()
                    .remove::<ShapeAssetPhysicsBinding>();
            }
        }
    }
}

pub fn cleanup_removed_shape_asset_bindings(
    mut commands: Commands,
    mut removed: RemovedComponents<ShapeAssetBinding>,
    materialized: Query<(), With<ShapeAssetPhysicsBinding>>,
) {
    for entity in removed.read() {
        if materialized.contains(entity) {
            commands
                .entity(entity)
                .remove::<PhysicsColliderSet>()
                .remove::<ShapeAssetPhysicsBinding>()
                .remove::<ShapeAssetPhysicsError>();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use az_asset::{
        ASSET_CATALOG_FILE_NAME, AssetCatalog as AssetCatalogFile,
        AssetCatalogEntry as AssetCatalogFileEntry, write_asset_catalog,
    };
    use az_core::AssetType;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::*;

    use super::*;
    use crate::{RockNRollAssetPlugin, RockNRollPlugin};

    #[cfg(feature = "render")]
    use crate::{
        BoxShape, MaterialFilter, PhysicalShape, RockNRollRenderPlugin, ShapeAssetChildren,
        ShapeAssetVisual, ShapeData,
    };

    fn shape_asset_id() -> AzAssetId {
        AzAssetId::new(uuid::uuid!("5F1668B9-7A80-4DA9-926C-7D7CFB90DBB5"), 9)
    }

    fn catalog_with_type(asset_type: AssetType) -> AssetCatalog {
        let temp = tempfile::tempdir().unwrap();
        let catalog = AssetCatalogFile::new(vec![AssetCatalogFileEntry::new(
            shape_asset_id(),
            *asset_type.as_uuid(),
            crate::SHAPE_ASSET_STABLE_NAME,
            crate::SHAPE_ASSET_VERSION,
            "objects/physics/barrier.rnr",
            Some(crate::SHAPE_ASSET_VERSION),
            0,
            [0; 32],
        )])
        .unwrap();
        write_asset_catalog(
            &catalog,
            File::create(temp.path().join(ASSET_CATALOG_FILE_NAME)).unwrap(),
        )
        .unwrap();
        AssetCatalog::open_native(temp.path()).unwrap()
    }

    #[test]
    fn plugin_binds_catalog_selected_shape_asset_identity() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins((RockNRollAssetPlugin, RockNRollPlugin));
        app.insert_resource(catalog_with_type(crate::SHAPE_ASSET_TYPE));

        let entity = app
            .world_mut()
            .spawn(ShapeAssetReference::from_id(shape_asset_id()))
            .id();

        app.update();

        let binding = app
            .world()
            .entity(entity)
            .get::<ShapeAssetBinding>()
            .unwrap();
        assert_eq!(binding.asset_id(), shape_asset_id());
    }

    #[test]
    fn plugin_rejects_catalog_entry_with_the_wrong_asset_type() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins((RockNRollAssetPlugin, RockNRollPlugin));
        app.insert_resource(catalog_with_type(AssetType::new(uuid::uuid!(
            "1FC869E8-9DA0-4A20-B912-45CA8BB665D1"
        ))));

        let entity = app
            .world_mut()
            .spawn(ShapeAssetReference::from_id(shape_asset_id()))
            .id();

        app.update();

        assert!(!app.world().entity(entity).contains::<ShapeAssetBinding>());
        assert!(matches!(
            app.world().entity(entity).get::<ShapeAssetReferenceError>(),
            Some(ShapeAssetReferenceError::Resolve(
                AssetRefLoadError::TypeMismatch { .. }
            ))
        ));
    }

    #[test]
    fn plugin_removes_binding_when_reference_clears() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins((RockNRollAssetPlugin, RockNRollPlugin));
        app.insert_resource(catalog_with_type(crate::SHAPE_ASSET_TYPE));

        let entity = app
            .world_mut()
            .spawn(ShapeAssetReference::from_id(shape_asset_id()))
            .id();

        app.update();
        assert!(app.world().entity(entity).contains::<ShapeAssetBinding>());

        app.world_mut()
            .entity_mut(entity)
            .insert(ShapeAssetReference::empty());
        app.update();

        assert!(!app.world().entity(entity).contains::<ShapeAssetBinding>());
    }

    #[test]
    #[cfg(feature = "render")]
    fn plugin_spawns_loaded_shape_asset_visuals() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .add_plugins((RockNRollAssetPlugin, RockNRollPlugin, RockNRollRenderPlugin));

        let handle = app
            .world_mut()
            .resource_mut::<Assets<ShapeAsset>>()
            .add(ShapeAsset::new(
                Box::new([]),
                MaterialFilter::default(),
                vec![PhysicalShape::new(
                    ShapeData::Box(BoxShape {
                        half_extents: Vec3::splat(0.5),
                        convex_radius: 0.0,
                    }),
                    Box::new([]),
                )]
                .into_boxed_slice(),
            ));
        let entity = app
            .world_mut()
            .spawn(ShapeAssetBinding::new(shape_asset_id(), handle))
            .id();

        app.update();

        let children = app
            .world()
            .entity(entity)
            .get::<ShapeAssetChildren>()
            .unwrap();
        assert_eq!(children.0.len(), 1);
        let child = app.world().entity(children.0[0]);
        assert!(child.contains::<ShapeAssetVisual>());
        assert!(child.contains::<Mesh3d>());
        assert!(child.contains::<MeshMaterial3d<StandardMaterial>>());
    }
}
