//! Native terrain-product to physics-heightfield conversion.
//!
//! Terrain cooking owns authoring and product formats. `RockNRoll` owns the
//! simulation-facing interpretation: the engine is Z-up, one cooked heightmap
//! becomes one static terrain body, and the world's height range converts the
//! stored `u16` samples into world-space heights.

use std::collections::HashSet;

use az_physics::{
    Axis3, BodyDescriptor, BodyKind, ColliderConfiguration, ColliderShape, PhysicsColliderSet,
    PhysicsPose, PhysicsSceneId, PhysicsSet,
};
use az_terrain_runtime::{
    TerrainHeightBinding, TerrainHeightmapAsset, TerrainRegionAsset, TerrainRegionBinding,
    TerrainRegionRef, TerrainWorldAsset,
};
use bevy::asset::{AssetEvent, AssetId};
use bevy::math::{Vec2, Vec3};
use bevy::prelude::*;
use thiserror::Error;

use crate::RockNRollAuthoringSet;

/// Borrowed inputs required to materialize one cooked terrain region.
#[derive(Debug, Clone, Copy)]
pub struct TerrainHeightFieldSource<'a> {
    world: &'a TerrainWorldAsset,
    region: &'a TerrainRegionRef,
    heightmap: &'a TerrainHeightmapAsset,
}

impl<'a> TerrainHeightFieldSource<'a> {
    #[must_use]
    pub const fn new(
        world: &'a TerrainWorldAsset,
        region: &'a TerrainRegionRef,
        heightmap: &'a TerrainHeightmapAsset,
    ) -> Self {
        Self {
            world,
            region,
            heightmap,
        }
    }

    #[must_use]
    pub const fn world(self) -> &'a TerrainWorldAsset {
        self.world
    }

    #[must_use]
    pub const fn region(self) -> &'a TerrainRegionRef {
        self.region
    }

    #[must_use]
    pub const fn heightmap(self) -> &'a TerrainHeightmapAsset {
        self.heightmap
    }
}

impl<'a>
    From<(
        &'a TerrainWorldAsset,
        &'a TerrainRegionRef,
        &'a TerrainHeightmapAsset,
    )> for TerrainHeightFieldSource<'a>
{
    fn from(
        (world, region, heightmap): (
            &'a TerrainWorldAsset,
            &'a TerrainRegionRef,
            &'a TerrainHeightmapAsset,
        ),
    ) -> Self {
        Self::new(world, region, heightmap)
    }
}

impl TryFrom<TerrainHeightFieldSource<'_>> for ColliderShape {
    type Error = TerrainHeightFieldError;

    fn try_from(source: TerrainHeightFieldSource<'_>) -> Result<Self, Self::Error> {
        validate_source(source)?;
        let range = source.world.height_range;
        let heights = source
            .heightmap
            .samples
            .iter()
            .copied()
            .map(|sample| range.decode_sample(sample))
            .collect();
        Ok(Self::HeightField {
            width: source.heightmap.width,
            length: source.heightmap.height,
            heights,
            aabb_min: Vec3::new(
                source.region.bounds.min.x,
                source.region.bounds.min.y,
                range.min,
            ),
            aabb_max: Vec3::new(
                source.region.bounds.max.x,
                source.region.bounds.max.y,
                range.max,
            ),
            up_axis: Axis3::Z,
        })
    }
}

impl TryFrom<TerrainHeightFieldSource<'_>> for ColliderConfiguration {
    type Error = TerrainHeightFieldError;

    fn try_from(source: TerrainHeightFieldSource<'_>) -> Result<Self, Self::Error> {
        let shape = ColliderShape::try_from(source)?;
        Ok(Self {
            shape,
            ..Self::default()
        })
    }
}

impl TryFrom<TerrainHeightFieldSource<'_>> for PhysicsColliderSet {
    type Error = TerrainHeightFieldError;

    fn try_from(source: TerrainHeightFieldSource<'_>) -> Result<Self, Self::Error> {
        Ok(Self(vec![ColliderConfiguration::try_from(source)?]))
    }
}

impl TryFrom<TerrainHeightFieldSource<'_>> for BodyDescriptor {
    type Error = TerrainHeightFieldError;

    fn try_from(source: TerrainHeightFieldSource<'_>) -> Result<Self, Self::Error> {
        let colliders = PhysicsColliderSet::try_from(source)?;
        Ok(Self {
            entity_id: None,
            pose: PhysicsPose::IDENTITY,
            kind: BodyKind::Static { terrain: true },
            colliders: colliders.0,
        })
    }
}

/// Borrowed constant-height region inputs.
#[derive(Debug, Clone, Copy)]
pub struct TerrainConstantHeightFieldSource<'a> {
    world: &'a TerrainWorldAsset,
    region: &'a TerrainRegionRef,
    height: f32,
}

impl<'a> TerrainConstantHeightFieldSource<'a> {
    #[must_use]
    pub const fn new(
        world: &'a TerrainWorldAsset,
        region: &'a TerrainRegionRef,
        height: f32,
    ) -> Self {
        Self {
            world,
            region,
            height,
        }
    }
}

impl<'a> From<(&'a TerrainWorldAsset, &'a TerrainRegionRef, f32)>
    for TerrainConstantHeightFieldSource<'a>
{
    fn from((world, region, height): (&'a TerrainWorldAsset, &'a TerrainRegionRef, f32)) -> Self {
        Self::new(world, region, height)
    }
}

impl TryFrom<TerrainConstantHeightFieldSource<'_>> for ColliderShape {
    type Error = TerrainHeightFieldError;

    fn try_from(source: TerrainConstantHeightFieldSource<'_>) -> Result<Self, Self::Error> {
        validate_domain(source.world, source.region)?;
        if !source.height.is_finite() {
            return Err(TerrainHeightFieldError::InvalidConstantHeight(
                source.height,
            ));
        }
        Ok(Self::HeightField {
            width: 2,
            length: 2,
            heights: vec![source.height; 4],
            aabb_min: Vec3::new(
                source.region.bounds.min.x,
                source.region.bounds.min.y,
                source.world.height_range.min,
            ),
            aabb_max: Vec3::new(
                source.region.bounds.max.x,
                source.region.bounds.max.y,
                source.world.height_range.max,
            ),
            up_axis: Axis3::Z,
        })
    }
}

impl TryFrom<TerrainConstantHeightFieldSource<'_>> for BodyDescriptor {
    type Error = TerrainHeightFieldError;

    fn try_from(source: TerrainConstantHeightFieldSource<'_>) -> Result<Self, Self::Error> {
        Ok(Self {
            entity_id: None,
            pose: PhysicsPose::IDENTITY,
            kind: BodyKind::Static { terrain: true },
            colliders: vec![ColliderConfiguration {
                shape: ColliderShape::try_from(source)?,
                ..ColliderConfiguration::default()
            }],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Error)]
pub enum TerrainHeightFieldError {
    #[error("terrain heightmap dimensions must both be at least 2, got {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error(
        "terrain heightmap sample count must be width * height, expected {expected}, got {actual}"
    )]
    InvalidSampleCount { expected: usize, actual: usize },
    #[error("terrain region bounds must be finite and non-empty, got {min:?}..{max:?}")]
    InvalidBounds { min: Vec2, max: Vec2 },
    #[error("terrain world height range must be finite and increasing, got {min}..{max}")]
    InvalidHeightRange { min: f32, max: f32 },
    #[error("terrain height spacing must be finite and positive, got {0}")]
    InvalidHeightSpacing(f32),
    #[error("terrain constant height must be finite, got {0}")]
    InvalidConstantHeight(f32),
}

fn validate_source(source: TerrainHeightFieldSource<'_>) -> Result<(), TerrainHeightFieldError> {
    let heightmap = source.heightmap;
    if heightmap.width < 2 || heightmap.height < 2 {
        return Err(TerrainHeightFieldError::InvalidDimensions {
            width: heightmap.width,
            height: heightmap.height,
        });
    }
    let expected = (heightmap.width as usize)
        .checked_mul(heightmap.height as usize)
        .ok_or(TerrainHeightFieldError::InvalidSampleCount {
            expected: usize::MAX,
            actual: heightmap.samples.len(),
        })?;
    if expected != heightmap.samples.len() {
        return Err(TerrainHeightFieldError::InvalidSampleCount {
            expected,
            actual: heightmap.samples.len(),
        });
    }
    validate_domain(source.world, source.region)
}

fn validate_domain(
    world: &TerrainWorldAsset,
    region: &TerrainRegionRef,
) -> Result<(), TerrainHeightFieldError> {
    let bounds = region.bounds;
    if !bounds.min.is_finite() || !bounds.max.is_finite() || !bounds.max.cmpgt(bounds.min).all() {
        return Err(TerrainHeightFieldError::InvalidBounds {
            min: bounds.min,
            max: bounds.max,
        });
    }
    let range = world.height_range;
    if !range.min.is_finite() || !range.max.is_finite() || range.max <= range.min {
        return Err(TerrainHeightFieldError::InvalidHeightRange {
            min: range.min,
            max: range.max,
        });
    }
    let spacing = world.resolution.height_spacing;
    if !spacing.is_finite() || spacing <= 0.0 {
        return Err(TerrainHeightFieldError::InvalidHeightSpacing(spacing));
    }
    Ok(())
}

/// Native terrain product identity currently materialized into a physics body.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerrainPhysicsBinding {
    pub world: AssetId<TerrainWorldAsset>,
    pub region: AssetId<TerrainRegionAsset>,
    pub heightmap: Option<AssetId<TerrainHeightmapAsset>>,
    pub scene: PhysicsSceneId,
}

/// Terrain-to-physics conversion failure retained on the region entity.
#[derive(Component, Debug, Clone, Copy, PartialEq, Error)]
pub enum TerrainPhysicsError {
    #[error("terrain owner {owner:?} has no explicit physics scene")]
    MissingPhysicsScene { owner: Entity },
    #[error(transparent)]
    HeightField(#[from] TerrainHeightFieldError),
}

pub fn configure(app: &mut App) {
    app.add_systems(
        Update,
        (materialize_terrain_physics, cleanup_removed_height_bindings)
            .chain()
            .in_set(RockNRollAuthoringSet::ShapeAssets)
            .in_set(PhysicsSet::Authoring),
    );
}

fn materialize_terrain_physics(
    mut commands: Commands,
    worlds: Option<Res<Assets<TerrainWorldAsset>>>,
    heightmaps: Option<Res<Assets<TerrainHeightmapAsset>>>,
    mut world_events: MessageReader<AssetEvent<TerrainWorldAsset>>,
    mut heightmap_events: MessageReader<AssetEvent<TerrainHeightmapAsset>>,
    owners: Query<&PhysicsSceneId>,
    regions: Query<(
        Entity,
        &TerrainRegionBinding,
        Ref<TerrainHeightBinding>,
        Option<&TerrainPhysicsBinding>,
    )>,
) {
    let (Some(worlds), Some(heightmaps)) = (worlds, heightmaps) else {
        return;
    };
    let changed_worlds = changed_assets(&mut world_events);
    let changed_heightmaps = changed_assets(&mut heightmap_events);

    for (entity, region, height, current) in &regions {
        let Ok(scene) = owners.get(region.owner()).copied() else {
            commands
                .entity(entity)
                .insert(TerrainPhysicsError::MissingPhysicsScene {
                    owner: region.owner(),
                })
                .remove::<BodyDescriptor>()
                .remove::<TerrainPhysicsBinding>();
            continue;
        };
        let world_id = region.world().id();
        let region_id = region.handle().id();
        let heightmap_id = match &*height {
            TerrainHeightBinding::Heightmap { handle, .. } => Some(handle.id()),
            TerrainHeightBinding::Constant { .. } => None,
        };
        let identity = TerrainPhysicsBinding {
            world: world_id,
            region: region_id,
            heightmap: heightmap_id,
            scene,
        };
        if !height.is_changed()
            && current == Some(&identity)
            && !changed_worlds.contains(&world_id)
            && heightmap_id.is_none_or(|id| !changed_heightmaps.contains(&id))
        {
            continue;
        }
        let Some(world) = worlds.get(world_id) else {
            if changed_worlds.contains(&world_id) {
                clear_terrain_physics(&mut commands, entity);
            }
            continue;
        };

        let descriptor = match &*height {
            TerrainHeightBinding::Heightmap { handle, .. } => {
                let Some(heightmap) = heightmaps.get(handle.id()) else {
                    if changed_heightmaps.contains(&handle.id()) {
                        clear_terrain_physics(&mut commands, entity);
                    }
                    continue;
                };
                BodyDescriptor::try_from(TerrainHeightFieldSource::new(
                    world,
                    region.region_ref(),
                    heightmap,
                ))
            }
            TerrainHeightBinding::Constant { value, .. } => BodyDescriptor::try_from(
                TerrainConstantHeightFieldSource::new(world, region.region_ref(), *value),
            ),
        };
        match descriptor {
            Ok(descriptor) => {
                commands
                    .entity(entity)
                    .insert((descriptor, identity, scene))
                    .remove::<TerrainPhysicsError>();
            }
            Err(error) => {
                commands
                    .entity(entity)
                    .insert(TerrainPhysicsError::from(error))
                    .remove::<BodyDescriptor>()
                    .remove::<TerrainPhysicsBinding>();
            }
        }
    }
}

fn cleanup_removed_height_bindings(
    mut commands: Commands,
    mut removed: RemovedComponents<TerrainHeightBinding>,
    materialized: Query<(), With<TerrainPhysicsBinding>>,
) {
    for entity in removed.read() {
        if materialized.contains(entity) {
            clear_terrain_physics(&mut commands, entity);
        }
    }
}

fn clear_terrain_physics(commands: &mut Commands, entity: Entity) {
    commands
        .entity(entity)
        .remove::<BodyDescriptor>()
        .remove::<PhysicsSceneId>()
        .remove::<TerrainPhysicsBinding>()
        .remove::<TerrainPhysicsError>();
}

fn changed_assets<A: Asset>(events: &mut MessageReader<AssetEvent<A>>) -> HashSet<AssetId<A>> {
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

#[cfg(test)]
mod tests {
    use az_terrain_runtime::{
        TerrainBounds, TerrainCoord, TerrainHeightImageSource, TerrainHeightRange,
        TerrainHeightSource, TerrainImageChannel, TerrainRegionAsset, TerrainResolution,
        TerrainWorldBinding, TerrainWorldReference, TerrainWorldRegions,
    };
    use bevy::asset::AssetPlugin;

    use crate::{RockNRollAssetPlugin, RockNRollPlugin};

    use super::*;

    fn world() -> TerrainWorldAsset {
        TerrainWorldAsset {
            name: "test".to_owned(),
            bounds: TerrainBounds {
                min: Vec2::ZERO,
                max: Vec2::splat(10.0),
            },
            height_range: TerrainHeightRange {
                min: -10.0,
                max: 90.0,
            },
            resolution: TerrainResolution {
                height_spacing: 5.0,
                surface_spacing: 5.0,
            },
            layers: String::new(),
            regions: Vec::new(),
        }
    }

    fn region() -> TerrainRegionRef {
        TerrainRegionRef {
            asset: "terrain/regions/test.azterrain-region.bin".to_owned(),
            coord: Some(TerrainCoord { x: 0, y: 0 }),
            bounds: TerrainBounds {
                min: Vec2::new(20.0, 30.0),
                max: Vec2::new(30.0, 40.0),
            },
            priority: 0,
        }
    }

    #[test]
    fn cooked_u16_samples_become_z_up_world_heights() {
        let world = world();
        let region = region();
        let heightmap = TerrainHeightmapAsset {
            name: "height".to_owned(),
            width: 2,
            height: 2,
            samples: vec![0, u16::MAX / 2, u16::MAX, u16::MAX],
        };

        let shape =
            ColliderShape::try_from(TerrainHeightFieldSource::new(&world, &region, &heightmap))
                .unwrap();

        let ColliderShape::HeightField {
            width,
            length,
            heights,
            aabb_min,
            aabb_max,
            up_axis,
        } = shape
        else {
            panic!("expected heightfield")
        };
        assert_eq!((width, length), (2, 2));
        assert_eq!(up_axis, Axis3::Z);
        assert_eq!(aabb_min, Vec3::new(20.0, 30.0, -10.0));
        assert_eq!(aabb_max, Vec3::new(30.0, 40.0, 90.0));
        assert!((heights[0] + 10.0).abs() < 0.001);
        assert!((heights[1] - 39.999_237).abs() < 0.001);
        assert!((heights[2] - 90.0).abs() < 0.001);
    }

    #[test]
    fn body_conversion_marks_static_terrain() {
        let world = world();
        let region = region();
        let heightmap = TerrainHeightmapAsset {
            name: "height".to_owned(),
            width: 2,
            height: 2,
            samples: vec![0; 4],
        };

        let body =
            BodyDescriptor::try_from(TerrainHeightFieldSource::new(&world, &region, &heightmap))
                .unwrap();

        assert_eq!(body.kind, BodyKind::Static { terrain: true });
        assert_eq!(body.colliders.len(), 1);
        assert!(body.colliders[0].simulated);
        assert!(body.colliders[0].in_scene_queries);
    }

    #[test]
    fn malformed_heightmap_is_rejected_before_allocation() {
        let world = world();
        let region = region();
        let heightmap = TerrainHeightmapAsset {
            name: "height".to_owned(),
            width: 2,
            height: 2,
            samples: vec![0; 3],
        };

        assert_eq!(
            ColliderShape::try_from(TerrainHeightFieldSource::new(&world, &region, &heightmap,)),
            Err(TerrainHeightFieldError::InvalidSampleCount {
                expected: 4,
                actual: 3,
            })
        );
    }

    #[test]
    fn constant_height_source_materializes_static_terrain() {
        let world = world();
        let region = region();

        let body =
            BodyDescriptor::try_from(TerrainConstantHeightFieldSource::new(&world, &region, 12.5))
                .unwrap();

        let ColliderShape::HeightField { heights, .. } = &body.colliders[0].shape else {
            panic!("expected heightfield")
        };
        assert_eq!(body.kind, BodyKind::Static { terrain: true });
        assert_eq!(heights, &[12.5; 4]);
    }

    /// A minimal app with the Rapier backend and both `RockNRoll` plugins, with
    /// `scene` already registered in the physics world.
    fn terrain_physics_app(scene: PhysicsSceneId) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .add_plugins((
                az_physics_rapier::RapierPhysicsPlugin::default(),
                RockNRollAssetPlugin,
                RockNRollPlugin,
            ));
        app.world_mut()
            .resource_mut::<az_physics::PhysicsWorld>()
            .ensure_scene(scene);
        app
    }

    /// Spawn a terrain world reference in `scene` and satisfy its world asset
    /// load with a one-region world.
    fn spawn_terrain_world(app: &mut App, scene: PhysicsSceneId) -> Entity {
        let world_entity = app
            .world_mut()
            .spawn((
                TerrainWorldReference::from("terrain/worlds/test.azterrain-world.bin"),
                scene,
            ))
            .id();
        app.update();

        let world_id = app
            .world()
            .entity(world_entity)
            .get::<TerrainWorldBinding>()
            .unwrap()
            .handle()
            .id();
        let mut world_asset = world();
        world_asset.regions.push(region());
        app.world_mut()
            .resource_mut::<Assets<TerrainWorldAsset>>()
            .insert(world_id, world_asset)
            .unwrap();
        app.update();
        app.update();
        world_entity
    }

    /// Satisfy the region asset load for the world's only region and return the
    /// region entity.
    fn insert_region_asset(app: &mut App, world_entity: Entity) -> Entity {
        let region_entity = app
            .world()
            .entity(world_entity)
            .get::<TerrainWorldRegions>()
            .unwrap()
            .entities()[0];
        let region_id = app
            .world()
            .entity(region_entity)
            .get::<TerrainRegionBinding>()
            .unwrap()
            .handle()
            .id();
        app.world_mut()
            .resource_mut::<Assets<TerrainRegionAsset>>()
            .insert(
                region_id,
                TerrainRegionAsset {
                    name: "test-region".to_owned(),
                    height: TerrainHeightSource::Image(TerrainHeightImageSource {
                        image: "terrain/heights/test.azterrain-height.bin".to_owned(),
                        channel: TerrainImageChannel::Red,
                        mip: 0,
                        tiling: Vec2::ONE,
                    }),
                    surface: None,
                    water: None,
                    layers: None,
                },
            )
            .unwrap();
        app.update();
        app.update();
        region_entity
    }

    /// Satisfy the heightmap asset load the region binding is waiting on.
    fn insert_heightmap_asset(app: &mut App, region_entity: Entity) {
        let heightmap_id = match app
            .world()
            .entity(region_entity)
            .get::<TerrainHeightBinding>()
            .unwrap()
        {
            TerrainHeightBinding::Heightmap { handle, .. } => handle.id(),
            TerrainHeightBinding::Constant { .. } => panic!("expected heightmap binding"),
        };
        app.world_mut()
            .resource_mut::<Assets<TerrainHeightmapAsset>>()
            .insert(
                heightmap_id,
                TerrainHeightmapAsset {
                    name: "test-height".to_owned(),
                    width: 2,
                    height: 2,
                    samples: vec![0, u16::MAX, 0, u16::MAX],
                },
            )
            .unwrap();
        app.update();
        app.update();
    }

    #[test]
    fn native_terrain_products_materialize_one_rapier_body_in_the_owner_scene() {
        let scene = PhysicsSceneId::new(9);
        let mut app = terrain_physics_app(scene);
        let world_entity = spawn_terrain_world(&mut app, scene);
        let region_entity = insert_region_asset(&mut app, world_entity);
        insert_heightmap_asset(&mut app, region_entity);

        let region = app.world().entity(region_entity);
        let binding = region.get::<TerrainPhysicsBinding>().unwrap();
        assert_eq!(binding.scene, scene);
        let body = *region.get::<az_physics::PhysicsBodyHandle>().unwrap();
        assert_eq!(body.scene(), scene);
        let status = app
            .world()
            .resource::<az_physics::PhysicsWorld>()
            .body_status(body)
            .unwrap();
        assert_eq!(status.simulation_class, az_physics::SimulationClass::Static);
    }
}
