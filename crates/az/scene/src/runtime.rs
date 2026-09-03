//! Runtime AZSCENE roots and exact instance ownership.

use std::collections::{BTreeMap, HashMap, HashSet};

use az_core::entity::LocalEntityRef;

use bevy::{
    asset::{AssetId, Assets, Handle},
    ecs::{
        hierarchy::ChildOf,
        reflect::{AppTypeRegistry, ReflectComponent},
        world::World,
    },
    prelude::Resource,
    reflect::Reflect,
};

use crate::{AzSceneAsset, AzSceneInstance, LocalEntityId, LocalEntityScopeId};

#[derive(Debug)]
struct AzSceneSourceScope {
    parent: Option<LocalEntityScopeId>,
    by_source_entity_id: HashMap<u64, bevy::prelude::Entity>,
}

#[derive(Debug, Clone, Copy)]
struct AzSceneMemberBinding {
    root: bevy::prelude::Entity,
    source_scope: LocalEntityScopeId,
}

#[derive(Debug)]
struct AzSceneRootScopes {
    source_scopes: Vec<AzSceneSourceScope>,
    members: Vec<bevy::prelude::Entity>,
}

/// Resolves native authored entity references from materialized source entities.
#[derive(Resource, Debug, Default)]
pub struct AzSceneEntityResolver {
    roots: HashMap<bevy::prelude::Entity, AzSceneRootScopes>,
    members: HashMap<bevy::prelude::Entity, AzSceneMemberBinding>,
}

impl AzSceneEntityResolver {
    /// Resolves a native source entity reference from the caller's authored scope.
    ///
    /// Invalid, stale, and out-of-scope references resolve to `None`.
    #[must_use]
    pub fn resolve_from(
        &self,
        source_entity: bevy::prelude::Entity,
        reference: LocalEntityRef,
    ) -> Option<bevy::prelude::Entity> {
        if reference.is_invalid() {
            return None;
        }
        let binding = *self.members.get(&source_entity)?;
        let root = self.roots.get(&binding.root)?;
        let mut source_scope = Some(binding.source_scope);
        let mut visited = HashSet::new();
        while let Some(scope_id) = source_scope {
            if !visited.insert(scope_id) {
                return None;
            }
            let scope = root
                .source_scopes
                .get(usize::try_from(scope_id.value()).ok()?)?;
            if let Some(entity) = scope
                .by_source_entity_id
                .get(&reference.entity_id.value())
                .copied()
            {
                return Some(entity);
            }
            source_scope = scope.parent;
        }
        None
    }

    fn register(
        &mut self,
        root: bevy::prelude::Entity,
        source_scopes: Vec<AzSceneSourceScope>,
        member_bindings: impl IntoIterator<Item = (bevy::prelude::Entity, LocalEntityScopeId)>,
    ) {
        self.unregister(root);
        let members = member_bindings
            .into_iter()
            .map(|(entity, source_scope)| {
                self.members
                    .insert(entity, AzSceneMemberBinding { root, source_scope });
                entity
            })
            .collect();
        self.roots.insert(
            root,
            AzSceneRootScopes {
                source_scopes,
                members,
            },
        );
    }

    fn unregister(&mut self, root: bevy::prelude::Entity) {
        let Some(scopes) = self.roots.remove(&root) else {
            return;
        };
        for member in scopes.members {
            if self
                .members
                .get(&member)
                .is_some_and(|binding| binding.root == root)
            {
                self.members.remove(&member);
            }
        }
    }
}

/// Runtime entity plus its processed native component identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AzSceneRuntimeComponentTarget {
    pub entity: bevy::prelude::Entity,
    pub native_type_id: uuid::Uuid,
    pub component_id: az_core::component::ComponentId,
}

/// Runtime request to materialize one processed AZSCENE product below this entity.
///
/// Replacing the handle replaces the isolated instance. Removing this
/// component or despawning its owner removes every entity materialized for the
/// instance. Runtime never reads a Prefab authoring document.
#[derive(bevy::prelude::Component, Debug, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct AzSceneRoot(pub Handle<AzSceneAsset>);

impl From<Handle<AzSceneAsset>> for AzSceneRoot {
    fn from(value: Handle<AzSceneAsset>) -> Self {
        Self(value)
    }
}

/// Exact ownership and product-local identity for one materialized entity.
///
/// Every entity materialized for an [`AzSceneRoot`] receives this component.
/// `root` identifies the requesting scene root that owns the isolated instance;
/// `local_entity_id` indexes that root's [`AzSceneInstanceSnapshot`].
#[derive(bevy::prelude::Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AzSceneInstanceMember {
    root: bevy::prelude::Entity,
    local_entity_id: LocalEntityId,
}

impl AzSceneInstanceMember {
    const fn new(root: bevy::prelude::Entity, local_entity_id: LocalEntityId) -> Self {
        Self {
            root,
            local_entity_id,
        }
    }

    /// The requesting [`AzSceneRoot`] that owns this entity's isolated instance.
    #[must_use]
    pub const fn root(self) -> bevy::prelude::Entity {
        self.root
    }

    /// This entity's stable product-local identity within the owning instance.
    #[must_use]
    pub const fn local_entity_id(self) -> LocalEntityId {
        self.local_entity_id
    }
}

/// Read-only identity and entity map for one live processed scene instance.
///
/// The component is attached to the requesting [`AzSceneRoot`] after
/// materialization and removed before that instance is torn down or replaced.
#[derive(bevy::prelude::Component, Debug, Clone, PartialEq, Eq)]
pub struct AzSceneInstanceSnapshot {
    asset_id: AssetId<AzSceneAsset>,
    entities: Vec<bevy::prelude::Entity>,
    roots: Vec<bevy::prelude::Entity>,
    by_source_alias: BTreeMap<String, bevy::prelude::Entity>,
    component_targets: Vec<AzSceneRuntimeComponentTarget>,
}

impl AzSceneInstanceSnapshot {
    fn new(
        asset_id: AssetId<AzSceneAsset>,
        asset: &AzSceneAsset,
        instance: &AzSceneInstance,
    ) -> Self {
        let by_source_alias = asset
            .metadata
            .entities
            .iter()
            .zip(instance.entities.iter().copied())
            .map(|(metadata, entity)| (metadata.source_alias.clone(), entity))
            .collect();
        let component_targets = asset
            .metadata
            .entities
            .iter()
            .zip(instance.entities.iter().copied())
            .flat_map(|(metadata, entity)| {
                metadata
                    .component_targets
                    .iter()
                    .map(move |target| AzSceneRuntimeComponentTarget {
                        entity,
                        native_type_id: target.native_type_id,
                        component_id: target.component_id,
                    })
            })
            .collect();
        Self {
            asset_id,
            entities: instance.entities.clone(),
            roots: instance.roots.clone(),
            by_source_alias,
            component_targets,
        }
    }

    #[must_use]
    pub const fn asset_id(&self) -> AssetId<AzSceneAsset> {
        self.asset_id
    }

    #[must_use]
    pub fn entities(&self) -> &[bevy::prelude::Entity] {
        &self.entities
    }

    #[must_use]
    pub fn roots(&self) -> &[bevy::prelude::Entity] {
        &self.roots
    }

    /// Resolves a product-local identity in this instance in constant time.
    #[must_use]
    pub fn entity(&self, local_entity_id: LocalEntityId) -> Option<bevy::prelude::Entity> {
        self.entities
            .get(usize::try_from(local_entity_id.value()).ok()?)
            .copied()
    }

    /// Resolves a flattened source alias within this isolated instance.
    #[must_use]
    pub fn entity_by_source_alias(&self, source_alias: &str) -> Option<bevy::prelude::Entity> {
        self.by_source_alias.get(source_alias).copied()
    }

    #[must_use]
    pub fn component_targets(&self) -> &[AzSceneRuntimeComponentTarget] {
        &self.component_targets
    }
}

/// Retained terminal failure for a requested scene instance.
#[derive(bevy::prelude::Component, Debug, Clone, PartialEq, Eq)]
pub struct AzSceneInstanceFailed {
    asset_id: AssetId<AzSceneAsset>,
    message: String,
}

impl AzSceneInstanceFailed {
    #[must_use]
    pub const fn asset_id(&self) -> AssetId<AzSceneAsset> {
        self.asset_id
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Debug)]
struct LiveAzSceneInstance {
    asset_id: AssetId<AzSceneAsset>,
    instance: AzSceneInstance,
}

#[derive(Resource, Debug, Default)]
pub struct AzSceneRuntime {
    live: HashMap<bevy::prelude::Entity, LiveAzSceneInstance>,
    failed: HashMap<bevy::prelude::Entity, AssetId<AzSceneAsset>>,
}

struct MaterializedAzSceneRoot {
    instance: AzSceneInstance,
    snapshot: AzSceneInstanceSnapshot,
    source_scopes: Vec<AzSceneSourceScope>,
    member_source_scopes: Vec<LocalEntityScopeId>,
}

fn remove_stale_scene_instances(
    world: &mut World,
    runtime: &mut AzSceneRuntime,
    resolver: &mut AzSceneEntityResolver,
    requested: &HashMap<bevy::prelude::Entity, AssetId<AzSceneAsset>>,
) {
    let stale = runtime
        .live
        .iter()
        .filter_map(|(entity, live)| {
            (requested.get(entity) != Some(&live.asset_id)).then_some(*entity)
        })
        .collect::<Vec<_>>();
    for entity in stale {
        resolver.unregister(entity);
        if let Some(mut live) = runtime.live.remove(&entity) {
            live.instance.remove(world);
        }
        if let Ok(mut root) = world.get_entity_mut(entity) {
            root.remove::<AzSceneInstanceSnapshot>();
            root.remove::<AzSceneInstanceFailed>();
        }
        runtime.failed.remove(&entity);
    }
}

fn materialize_scene_root(
    world: &mut World,
    asset_id: AssetId<AzSceneAsset>,
    registry: &bevy::reflect::TypeRegistry,
) -> Option<Result<MaterializedAzSceneRoot, crate::AzSceneMaterializeError>> {
    world.resource_scope(
        |world, scenes: bevy::prelude::Mut<'_, Assets<AzSceneAsset>>| {
            scenes.get(asset_id).map(|scene| {
                scene.materialize(world, registry).map(|instance| {
                    let snapshot = AzSceneInstanceSnapshot::new(asset_id, scene, &instance);
                    let mut source_scopes = scene
                        .metadata
                        .source_scopes
                        .iter()
                        .map(|scope| AzSceneSourceScope {
                            parent: scope.parent,
                            by_source_entity_id: HashMap::new(),
                        })
                        .collect::<Vec<_>>();
                    let member_source_scopes = scene
                        .metadata
                        .entities
                        .iter()
                        .zip(instance.entities.iter().copied())
                        .map(|(metadata, entity)| {
                            if let Some(source_entity_id) = metadata.source_entity_id {
                                source_scopes[usize::try_from(metadata.source_scope.value())
                                    .expect("validated source scope must fit usize")]
                                .by_source_entity_id
                                .insert(source_entity_id.value(), entity);
                            }
                            metadata.source_scope
                        })
                        .collect();
                    MaterializedAzSceneRoot {
                        instance,
                        snapshot,
                        source_scopes,
                        member_source_scopes,
                    }
                })
            })
        },
    )
}

fn attach_materialized_scene_root(
    world: &mut World,
    resolver: &mut AzSceneEntityResolver,
    root_entity: bevy::prelude::Entity,
    materialized: MaterializedAzSceneRoot,
) -> AzSceneInstance {
    let MaterializedAzSceneRoot {
        instance,
        snapshot,
        source_scopes,
        member_source_scopes,
    } = materialized;
    for (index, entity) in instance.entities.iter().copied().enumerate() {
        let local_entity_id = LocalEntityId::new(
            u32::try_from(index).expect("validated AZSCENE entity count must fit LocalEntityId"),
        );
        world
            .entity_mut(entity)
            .insert(AzSceneInstanceMember::new(root_entity, local_entity_id));
    }
    for entity in &instance.roots {
        world.entity_mut(*entity).insert(ChildOf(root_entity));
    }
    resolver.register(
        root_entity,
        source_scopes,
        instance.entities.iter().copied().zip(member_source_scopes),
    );
    world
        .entity_mut(root_entity)
        .insert(snapshot)
        .remove::<AzSceneInstanceFailed>();
    instance
}

pub fn materialize_az_scene_roots(world: &mut World) {
    let roots = {
        let mut query = world.query::<(bevy::prelude::Entity, &AzSceneRoot)>();
        query
            .iter(world)
            .map(|(entity, root)| (entity, root.0.clone()))
            .collect::<Vec<_>>()
    };
    let requested = roots
        .iter()
        .map(|(entity, handle)| (*entity, handle.id()))
        .collect::<HashMap<_, _>>();
    let mut runtime = world
        .remove_resource::<AzSceneRuntime>()
        .unwrap_or_default();
    let mut resolver = world
        .remove_resource::<AzSceneEntityResolver>()
        .unwrap_or_default();
    remove_stale_scene_instances(world, &mut runtime, &mut resolver, &requested);

    let registry = world.resource::<AppTypeRegistry>().0.clone();
    for (root_entity, handle) in roots {
        let asset_id = handle.id();
        if runtime.live.contains_key(&root_entity)
            || runtime.failed.get(&root_entity) == Some(&asset_id)
        {
            continue;
        }

        let Some(result) = materialize_scene_root(world, asset_id, &registry.read()) else {
            continue;
        };
        match result {
            Ok(materialized) => {
                let instance =
                    attach_materialized_scene_root(world, &mut resolver, root_entity, materialized);
                tracing::info!(
                    root = ?root_entity,
                    ?asset_id,
                    entity_count = instance.entities.len(),
                    "materialized AZSCENE instance"
                );
                runtime
                    .live
                    .insert(root_entity, LiveAzSceneInstance { asset_id, instance });
                runtime.failed.remove(&root_entity);
            }
            Err(error) => {
                resolver.unregister(root_entity);
                tracing::error!(
                    root = ?root_entity,
                    ?asset_id,
                    %error,
                    "failed to materialize AZSCENE instance"
                );
                runtime.failed.insert(root_entity, asset_id);
                world.entity_mut(root_entity).insert(AzSceneInstanceFailed {
                    asset_id,
                    message: error.to_string(),
                });
            }
        }
    }

    runtime
        .failed
        .retain(|entity, asset_id| requested.get(entity) == Some(asset_id));
    world.insert_resource(resolver);
    world.insert_resource(runtime);
}

#[cfg(test)]
mod tests {
    use bevy::{
        asset::{AssetPlugin, Assets},
        ecs::schedule::IntoScheduleConfigs,
        prelude::{App, Entity, MinimalPlugins, With, World},
        world_serialization::{DynamicWorld, DynamicWorldBuilder},
    };

    use az_core::{EntityId, entity::LocalEntityRef};

    use super::*;
    use crate::{
        AzSceneEntityMetadata, AzSceneEntityResolver, AzSceneMetadata, AzSceneSourceScopeMetadata,
        LocalEntityScopeId,
    };

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default(), crate::AzScenePlugin))
            .register_type::<Entity>();
        app
    }

    fn asset_with_aliases(app: &App, aliases: &[&str]) -> AzSceneAsset {
        let mut source = World::new();
        let entities = aliases
            .iter()
            .map(|_| source.spawn_empty().id())
            .collect::<Vec<_>>();
        let dynamic_world = {
            let registry = app.world().resource::<AppTypeRegistry>().read();
            DynamicWorldBuilder::from_world(&source, &registry)
                .extract_entities(entities.iter().copied())
                .build()
        };
        let metadata = AzSceneMetadata {
            source_scopes: vec![AzSceneSourceScopeMetadata { parent: None }],
            entities: aliases
                .iter()
                .map(|alias| AzSceneEntityMetadata {
                    source_alias: (*alias).to_owned(),
                    source_scope: LocalEntityScopeId::ROOT,
                    source_entity_id: None,
                    parent: None,
                    component_targets: Vec::new(),
                })
                .collect(),
        };
        AzSceneAsset::new_in_entity_order(dynamic_world, &entities, metadata)
            .expect("test AZSCENE should preserve canonical entity order")
    }

    #[test]
    fn sibling_source_entities_resolve_only_within_their_own_scope() {
        let mut app = test_app();
        let shared_target_id = EntityId::new(9_101);
        let mut asset = asset_with_aliases(
            &app,
            &["first/host", "first/target", "second/host", "second/target"],
        );
        asset.metadata.source_scopes.extend([
            AzSceneSourceScopeMetadata {
                parent: Some(LocalEntityScopeId::ROOT),
            },
            AzSceneSourceScopeMetadata {
                parent: Some(LocalEntityScopeId::ROOT),
            },
        ]);
        for (index, scope) in [1_u32, 1, 2, 2].into_iter().enumerate() {
            asset.metadata.entities[index].source_scope = LocalEntityScopeId::new(scope);
        }
        asset.metadata.entities[1].source_entity_id = Some(shared_target_id);
        asset.metadata.entities[3].source_entity_id = Some(shared_target_id);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let root = app.world_mut().spawn(AzSceneRoot(handle)).id();

        app.update();

        let entities = app
            .world()
            .get::<AzSceneInstanceSnapshot>(root)
            .expect("scene materialized")
            .entities()
            .to_vec();
        let resolver = app.world().resource::<AzSceneEntityResolver>();
        assert_eq!(
            resolver.resolve_from(entities[0], LocalEntityRef::new(shared_target_id)),
            Some(entities[1])
        );
        assert_eq!(
            resolver.resolve_from(entities[2], LocalEntityRef::new(shared_target_id)),
            Some(entities[3])
        );
    }

    #[test]
    fn nested_source_scope_resolves_parent_scope_within_the_same_product() {
        let mut app = test_app();
        let parent_target_id = EntityId::new(7_101);
        let mut asset = asset_with_aliases(&app, &["nested/host", "root/target"]);
        asset
            .metadata
            .source_scopes
            .push(AzSceneSourceScopeMetadata {
                parent: Some(LocalEntityScopeId::ROOT),
            });
        asset.metadata.entities[0].source_scope = LocalEntityScopeId::new(1);
        asset.metadata.entities[1].source_entity_id = Some(parent_target_id);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let root = app.world_mut().spawn(AzSceneRoot(handle)).id();

        app.update();

        let entities = app
            .world()
            .get::<AzSceneInstanceSnapshot>(root)
            .expect("scene materialized")
            .entities()
            .to_vec();
        assert_eq!(
            app.world()
                .resource::<AzSceneEntityResolver>()
                .resolve_from(entities[0], LocalEntityRef::new(parent_target_id)),
            Some(entities[1])
        );
    }

    #[test]
    fn materialized_source_ids_resolve_within_their_live_root() {
        let mut app = test_app();
        let source_entity_id = EntityId::new(9_001);
        let mut asset = asset_with_aliases(&app, &["host", "target"]);
        asset.metadata.entities[1].source_entity_id = Some(source_entity_id);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let root = app.world_mut().spawn(AzSceneRoot(handle)).id();

        app.update();

        let target = app
            .world()
            .get::<AzSceneInstanceSnapshot>(root)
            .expect("scene materialized")
            .entity(LocalEntityId::new(1))
            .expect("target entity");
        let resolver = app.world().resource::<AzSceneEntityResolver>();
        assert_eq!(
            resolver.resolve_from(target, LocalEntityRef::new(source_entity_id)),
            Some(target)
        );
        assert_eq!(
            resolver.resolve_from(target, LocalEntityRef::invalid()),
            None
        );
        assert_eq!(
            resolver.resolve_from(target, LocalEntityRef::new(EntityId::new(9_999))),
            None
        );
    }

    #[test]
    fn dynamic_child_root_cannot_resolve_source_ids_from_its_runtime_parent() {
        let mut app = test_app();
        let source_entity_id = EntityId::new(7_001);
        let mut parent_asset = asset_with_aliases(&app, &["parent-target"]);
        parent_asset.metadata.entities[0].source_entity_id = Some(source_entity_id);
        let parent_handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(parent_asset);
        let child_asset = asset_with_aliases(&app, &["child"]);
        let child_handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(child_asset);
        let parent_root = app.world_mut().spawn(AzSceneRoot(parent_handle)).id();

        app.update();

        let parent_target = app
            .world()
            .get::<AzSceneInstanceSnapshot>(parent_root)
            .expect("parent scene materialized")
            .entities()[0];
        let child_root = app
            .world_mut()
            .spawn((AzSceneRoot(child_handle), ChildOf(parent_target)))
            .id();

        app.update();

        assert_eq!(
            app.world()
                .resource::<AzSceneEntityResolver>()
                .resolve_from(
                    app.world()
                        .get::<AzSceneInstanceSnapshot>(child_root)
                        .expect("child scene materialized")
                        .entities()[0],
                    LocalEntityRef::new(source_entity_id),
                ),
            None
        );
    }

    #[test]
    fn duplicate_source_ids_fail_materialization_without_registering_a_scope() {
        let mut app = test_app();
        let duplicate_source_id = EntityId::new(8_001);
        let mut asset = asset_with_aliases(&app, &["first", "second"]);
        for metadata in &mut asset.metadata.entities {
            metadata.source_entity_id = Some(duplicate_source_id);
        }
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let root = app.world_mut().spawn(AzSceneRoot(handle)).id();

        app.update();

        assert!(app.world().get::<AzSceneInstanceFailed>(root).is_some());
        assert!(app.world().get::<AzSceneInstanceSnapshot>(root).is_none());
        assert_eq!(
            app.world()
                .resource::<AzSceneEntityResolver>()
                .resolve_from(root, LocalEntityRef::new(duplicate_source_id)),
            None
        );
    }

    #[test]
    fn root_materializes_and_removes_owned_instance() {
        let mut app = test_app();
        let asset = AzSceneAsset::new(DynamicWorld::default(), AzSceneMetadata::default());
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let root = app.world_mut().spawn(AzSceneRoot(handle.clone())).id();

        app.update();
        assert!(app.world().get_entity(root).is_ok());
        assert!(
            app.world()
                .resource::<AzSceneRuntime>()
                .live
                .contains_key(&root)
        );
        let snapshot = app
            .world()
            .get::<AzSceneInstanceSnapshot>(root)
            .expect("root should expose materialized instance identity");
        assert_eq!(snapshot.asset_id(), handle.id());
        assert!(snapshot.entities().is_empty());
        assert!(snapshot.roots().is_empty());

        app.world_mut().entity_mut(root).remove::<AzSceneRoot>();
        app.update();
        assert!(
            !app.world()
                .resource::<AzSceneRuntime>()
                .live
                .contains_key(&root)
        );
        assert!(app.world().get::<AzSceneInstanceSnapshot>(root).is_none());
    }

    #[test]
    fn identical_source_ids_stay_isolated_between_live_roots_and_teardown() {
        let mut app = test_app();
        let aliases = ["shared-root", "shared-child"];
        let shared_source_id = EntityId::new(5_001);
        let mut asset = asset_with_aliases(&app, &aliases);
        asset.metadata.entities[1].source_entity_id = Some(shared_source_id);
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let first_root = app.world_mut().spawn(AzSceneRoot(handle.clone())).id();
        let second_root = app.world_mut().spawn(AzSceneRoot(handle)).id();

        app.update();

        let first_snapshot = app
            .world()
            .get::<AzSceneInstanceSnapshot>(first_root)
            .expect("first root should expose its instance snapshot");
        let second_snapshot = app
            .world()
            .get::<AzSceneInstanceSnapshot>(second_root)
            .expect("second root should expose its instance snapshot");
        let mut first_entities = Vec::new();
        let mut second_entities = Vec::new();
        for (index, alias) in aliases.iter().enumerate() {
            let local_entity_id = LocalEntityId::new(
                u32::try_from(index).expect("test entity index must fit LocalEntityId"),
            );
            let first = first_snapshot
                .entity(local_entity_id)
                .expect("first instance should resolve its local entity");
            let second = second_snapshot
                .entity(local_entity_id)
                .expect("second instance should resolve its local entity");

            assert_ne!(first, second);
            assert_eq!(first_snapshot.entity_by_source_alias(alias), Some(first));
            assert_eq!(second_snapshot.entity_by_source_alias(alias), Some(second));
            let first_member = app
                .world()
                .get::<AzSceneInstanceMember>(first)
                .expect("first entity should record exact instance membership");
            let second_member = app
                .world()
                .get::<AzSceneInstanceMember>(second)
                .expect("second entity should record exact instance membership");
            assert_eq!(
                (first_member.root(), first_member.local_entity_id()),
                (first_root, local_entity_id)
            );
            assert_eq!(
                (second_member.root(), second_member.local_entity_id()),
                (second_root, local_entity_id)
            );
            first_entities.push(first);
            second_entities.push(second);
        }
        let missing_local_entity_id = LocalEntityId::new(
            u32::try_from(aliases.len()).expect("test entity count must fit LocalEntityId"),
        );
        assert_eq!(first_snapshot.entity(missing_local_entity_id), None);
        assert_eq!(second_snapshot.entity(missing_local_entity_id), None);
        let reference = LocalEntityRef::new(shared_source_id);
        let resolver = app.world().resource::<AzSceneEntityResolver>();
        assert_eq!(
            resolver.resolve_from(first_entities[0], reference),
            Some(first_entities[1])
        );
        assert_eq!(
            resolver.resolve_from(second_entities[0], reference),
            Some(second_entities[1])
        );

        app.world_mut()
            .entity_mut(first_root)
            .remove::<AzSceneRoot>();
        app.update();

        assert!(
            first_entities
                .iter()
                .all(|entity| app.world().get_entity(*entity).is_err())
        );
        assert!(second_entities.iter().all(|entity| {
            app.world()
                .get::<AzSceneInstanceMember>(*entity)
                .is_some_and(|member| member.root() == second_root)
        }));
        let resolver = app.world().resource::<AzSceneEntityResolver>();
        assert_eq!(resolver.resolve_from(first_entities[0], reference), None);
        assert_eq!(
            resolver.resolve_from(second_entities[0], reference),
            Some(second_entities[1])
        );
    }

    #[derive(Resource, Default)]
    struct ResolutionObserved(Option<bevy::prelude::Entity>);

    fn observe_resolution_after_materialization(world: &mut World) {
        let root = {
            let mut roots = world.query_filtered::<bevy::prelude::Entity, With<AzSceneRoot>>();
            roots.iter(world).next()
        };
        let resolved = root.and_then(|root| {
            world
                .get::<AzSceneInstanceSnapshot>(root)
                .and_then(|snapshot| snapshot.entities().first().copied())
                .and_then(|source| {
                    world
                        .resource::<AzSceneEntityResolver>()
                        .resolve_from(source, LocalEntityRef::new(EntityId::new(6_001)))
                })
        });
        world.resource_mut::<ResolutionObserved>().0 = resolved;
    }

    #[test]
    fn resolver_registration_is_visible_after_materialization_set() {
        let mut app = test_app();
        app.init_resource::<ResolutionObserved>().add_systems(
            bevy::app::Update,
            observe_resolution_after_materialization.after(crate::AzSceneMaterializationSet),
        );
        let mut asset = asset_with_aliases(&app, &["target"]);
        asset.metadata.entities[0].source_entity_id = Some(EntityId::new(6_001));
        let handle = app
            .world_mut()
            .resource_mut::<Assets<AzSceneAsset>>()
            .add(asset);
        let root = app.world_mut().spawn(AzSceneRoot(handle)).id();

        app.update();

        let target = app
            .world()
            .get::<AzSceneInstanceSnapshot>(root)
            .expect("scene materialized")
            .entities()[0];
        assert_eq!(app.world().resource::<ResolutionObserved>().0, Some(target));
    }
}
