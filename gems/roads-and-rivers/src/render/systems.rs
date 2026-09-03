//! Road and river mesh render systems.

use az_gem_lmbr_central::SplineComponent;
use bevy::prelude::*;

use crate::components::{RiverComponent, RoadComponent};

use super::binding::{RoadsAndRiversMaterialBinding, sync_material_asset_binding};
use super::config::{RoadsAndRiversRenderConfig, river_material, road_material};

// Bevy system parameters are owned wrappers: `Res`/`ResMut`/`Commands` taken
// by reference no longer satisfy `IntoSystem`, so the system stops registering.
#[allow(clippy::type_complexity, clippy::needless_pass_by_value)]
pub(super) fn render_road_components(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    config: Res<RoadsAndRiversRenderConfig>,
    query: Query<
        (
            Entity,
            &RoadComponent,
            &SplineComponent,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&RoadsAndRiversMaterialBinding>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(
            Changed<RoadComponent>,
            Changed<SplineComponent>,
            Without<Mesh3d>,
        )>,
    >,
) {
    for (entity, road, spline, material, binding, transform, name) in &query {
        let mesh = meshes.add(road.mesh(spline.configuration.spline.data()));
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(Mesh3d(mesh));

        let material_changed = sync_material_asset_binding(
            &mut entity_commands,
            asset_server.as_deref(),
            &road.material_path,
            binding,
        );
        if material.is_none() || material_changed {
            entity_commands.insert(MeshMaterial3d(materials.add(road_material(&config))));
        }
        if transform.is_none() {
            entity_commands.insert(Transform::default());
        }
        if name.is_none() {
            entity_commands.insert(Name::new("Road"));
        }
    }
}

// Bevy system parameters must stay owned; see `render_road_components`.
#[allow(clippy::type_complexity, clippy::needless_pass_by_value)]
pub(super) fn render_river_components(
    mut commands: Commands,
    asset_server: Option<Res<AssetServer>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    config: Res<RoadsAndRiversRenderConfig>,
    query: Query<
        (
            Entity,
            &RiverComponent,
            &SplineComponent,
            Option<&MeshMaterial3d<StandardMaterial>>,
            Option<&RoadsAndRiversMaterialBinding>,
            Option<&Transform>,
            Option<&Name>,
        ),
        Or<(
            Changed<RiverComponent>,
            Changed<SplineComponent>,
            Without<Mesh3d>,
        )>,
    >,
) {
    for (entity, river, spline, material, binding, transform, name) in &query {
        let mesh = meshes.add(river.mesh(spline.configuration.spline.data()));
        let mut entity_commands = commands.entity(entity);
        entity_commands.insert(Mesh3d(mesh));

        let material_changed = sync_material_asset_binding(
            &mut entity_commands,
            asset_server.as_deref(),
            &river.material_path,
            binding,
        );
        if material.is_none() || material_changed {
            entity_commands.insert(MeshMaterial3d(materials.add(river_material(&config))));
        }
        if transform.is_none() {
            entity_commands.insert(Transform::default());
        }
        if name.is_none() {
            entity_commands.insert(Name::new("River"));
        }
    }
}
