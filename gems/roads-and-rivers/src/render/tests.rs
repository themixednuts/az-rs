use az_gem_lmbr_central::{LinearSpline, Spline, SplineCommon, SplineComponent, SplineData};
use bevy::prelude::*;

use super::*;
use crate::components::{RiverComponent, RoadComponent};
use crate::render::config::road_material;
use crate::{RoadsAndRiversPlugin, SplineGeometry};
#[test]
fn plugin_renders_road_components_as_meshes() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(RoadsAndRiversPlugin);

    let entity = app
        .world_mut()
        .spawn((
            spline_component([Vec3::ZERO, Vec3::Z * 8.0]),
            RoadComponent {
                geometry: SplineGeometry {
                    segment_length: 2.0,
                    ..default()
                },
                ..default()
            },
        ))
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<Mesh3d>());
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert!(entity_ref.contains::<Transform>());
}

#[test]
fn plugin_renders_river_components_as_meshes() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_plugins(RoadsAndRiversPlugin);

    let entity = app
        .world_mut()
        .spawn((
            spline_component([Vec3::ZERO, Vec3::X * 8.0]),
            RiverComponent {
                geometry: SplineGeometry {
                    segment_length: 2.0,
                    ..default()
                },
                ..default()
            },
        ))
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(entity_ref.contains::<Mesh3d>());
    assert!(entity_ref.contains::<MeshMaterial3d<StandardMaterial>>());
    assert!(entity_ref.contains::<Transform>());
}

// The asserted alpha is the opacity literal fed to `material_asset_with_opacity`
// and carried through unchanged, so the values are bit-identical.
#[allow(clippy::float_cmp)]
#[test]
fn material_asset_binding_applies_loaded_material() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<AssetEvent<MaterialAsset>>()
        .init_resource::<Assets<MaterialAsset>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_systems(Update, apply_material_asset_bindings);

    let material_asset_handle = app
        .world_mut()
        .resource_mut::<Assets<MaterialAsset>>()
        .add(material_asset_with_opacity(0.75));
    let fallback_handle = app
        .world_mut()
        .resource_mut::<Assets<StandardMaterial>>()
        .add(road_material(&RoadsAndRiversRenderConfig::default()));
    let entity = app
        .world_mut()
        .spawn((
            RoadsAndRiversMaterialBinding::new(
                "materials/road/defaultroad.mtl",
                material_asset_handle,
            ),
            MeshMaterial3d(fallback_handle.clone()),
        ))
        .id();

    app.update();

    let entity_ref = app.world().entity(entity);
    let binding = entity_ref.get::<RoadsAndRiversMaterialBinding>().unwrap();
    assert!(binding.is_applied());

    let material_handle = entity_ref
        .get::<MeshMaterial3d<StandardMaterial>>()
        .unwrap()
        .0
        .clone();
    assert_ne!(material_handle, fallback_handle);

    let materials = app.world().resource::<Assets<StandardMaterial>>();
    let material = materials.get(&material_handle).unwrap();
    assert_eq!(material.base_color.to_srgba().alpha, 0.75);
}

// As above: the asserted alpha is the opacity literal the modified asset was
// built with, carried through unchanged.
#[allow(clippy::float_cmp)]
#[test]
fn material_asset_binding_reapplies_modified_material() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_message::<AssetEvent<MaterialAsset>>()
        .init_resource::<Assets<MaterialAsset>>()
        .init_resource::<Assets<StandardMaterial>>()
        .add_systems(Update, apply_material_asset_bindings);

    let material_asset_handle = app
        .world_mut()
        .resource_mut::<Assets<MaterialAsset>>()
        .add(material_asset_with_opacity(0.75));
    let entity = app
        .world_mut()
        .spawn(RoadsAndRiversMaterialBinding::new(
            "materials/road/defaultroad.mtl",
            material_asset_handle.clone(),
        ))
        .id();

    app.update();
    let first_material_handle = app
        .world()
        .entity(entity)
        .get::<MeshMaterial3d<StandardMaterial>>()
        .unwrap()
        .0
        .clone();

    app.world_mut()
        .resource_mut::<Assets<MaterialAsset>>()
        .get_mut(&material_asset_handle)
        .unwrap()
        .root
        .opacity = 0.35;
    app.world_mut().write_message(AssetEvent::Modified {
        id: material_asset_handle.id(),
    });
    app.update();

    let entity_ref = app.world().entity(entity);
    assert!(
        entity_ref
            .get::<RoadsAndRiversMaterialBinding>()
            .unwrap()
            .is_applied()
    );

    let material_handle = entity_ref
        .get::<MeshMaterial3d<StandardMaterial>>()
        .unwrap()
        .0
        .clone();
    assert_ne!(material_handle, first_material_handle);

    let materials = app.world().resource::<Assets<StandardMaterial>>();
    let material = materials.get(&material_handle).unwrap();
    assert_eq!(material.base_color.to_srgba().alpha, 0.35);
}

fn material_asset_with_opacity(opacity: f32) -> MaterialAsset {
    MaterialAsset {
        root: az_gem_lmbr_central::MaterialDefinition {
            diffuse: Some(bevy::color::Srgba::new(0.4, 0.5, 0.6, 1.0)),
            opacity,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn spline_component(vertices: impl IntoIterator<Item = Vec3>) -> SplineComponent {
    SplineComponent {
        configuration: SplineCommon {
            spline: Spline::Linear(LinearSpline {
                spline: SplineData {
                    vertices: vertices.into_iter().collect(),
                    closed: false,
                },
            }),
        },
        ..default()
    }
}
