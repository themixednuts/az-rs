use az_gem_lmbr_central::sync_material_asset_binding;
use bevy::asset::AssetEvent;
use bevy::mesh::Mesh3d;
use bevy::prelude::*;

use super::config::{TerrainRegionRenderConfig, TerrainWaterRenderConfig};
use super::material::terrain_water_material;
use super::resources::{LoadedTerrainWorldManifests, RenderedTerrainRegions};
use crate::engine_asset;
use crate::region::{TerrainRegionAsset, TerrainSurfacePalette, TerrainWaterSurface};
use crate::world::TerrainWorld;

// Bevy system parameters are owned wrappers: `Res`/`ResMut`/`Commands` taken
// by reference no longer satisfy `IntoSystem`, so the system stops registering.
#[allow(clippy::too_many_arguments, clippy::needless_pass_by_value)]
pub(super) fn spawn_loaded_terrain_regions(
    mut commands: Commands,
    terrain_world: Option<Res<TerrainWorld>>,
    terrain_assets: Res<Assets<TerrainRegionAsset>>,
    asset_server: Option<Res<AssetServer>>,
    render_config: Res<TerrainRegionRenderConfig>,
    water_config: Res<TerrainWaterRenderConfig>,
    mut rendered: ResMut<RenderedTerrainRegions>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(terrain_world) = terrain_world else {
        return;
    };

    for (region, handle) in &terrain_world.loaded_regions {
        if rendered.regions.contains(region) {
            continue;
        }

        let Some(asset) = terrain_assets.get(handle) else {
            continue;
        };
        let mesh_resolution = render_config
            .mesh_resolution
            .resolve(asset.heightmap.resolution);
        let Ok(bundle) = asset.render_bundle_with_resolution(mesh_resolution) else {
            continue;
        };
        let surface_palette = TerrainSurfacePalette::from_material_layers(
            render_config.base_color,
            &asset.material_layers,
        );
        let material = materials.add(StandardMaterial {
            base_color: if surface_palette.is_some() {
                Color::WHITE
            } else {
                render_config.base_color
            },
            perceptual_roughness: 0.92,
            ..Default::default()
        });

        let mut entity_commands = commands.spawn((
            Name::new(format!("Terrain Region {},{}", region.x, region.y)),
            bundle,
            MeshMaterial3d(material),
        ));
        if let Some(surface_map) = asset.surface_map() {
            entity_commands.insert(surface_map);
        }
        if let Some(surface_palette) = surface_palette {
            entity_commands.insert(surface_palette);
        }
        if let Some(material_path) = asset.render_material_path() {
            sync_material_asset_binding(
                &mut entity_commands,
                asset_server.as_deref(),
                material_path,
                None,
            );
        }

        if let Some(mesh) = asset.water_surface_mesh(water_config.surface_offset) {
            let region_size = asset
                .water_region_world_size()
                .unwrap_or_else(|| asset.region_world_size());
            let center = Vec3::new(
                region_size.mul_add(0.5, asset.origin.x),
                0.0,
                region_size.mul_add(0.5, asset.origin.y),
            );
            let material = materials.add(terrain_water_material(&water_config));
            commands.spawn((
                Name::new(format!("Terrain Water {},{}", region.x, region.y)),
                TerrainWaterSurface { region: *region },
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(material),
                Transform::from_translation(center),
            ));
        }

        rendered.regions.insert(*region);
    }
}

// Bevy system parameters must stay owned; see `spawn_loaded_terrain_regions`.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn process_loaded_terrain_world_manifests(
    mut events: MessageReader<AssetEvent<engine_asset::TerrainWorldManifest>>,
    manifests: Res<Assets<engine_asset::TerrainWorldManifest>>,
    asset_server: Res<AssetServer>,
    mut terrain_world: ResMut<TerrainWorld>,
    mut loaded: ResMut<LoadedTerrainWorldManifests>,
) {
    for event in events.read() {
        let id = match event {
            AssetEvent::Added { id } | AssetEvent::LoadedWithDependencies { id } => *id,
            AssetEvent::Modified { .. }
            | AssetEvent::Removed { .. }
            | AssetEvent::Unused { .. } => {
                continue;
            }
        };

        if !loaded.manifests.insert(id) {
            continue;
        }

        let Some(manifest) = manifests.get(id) else {
            continue;
        };
        if !manifest.world_name.is_empty() {
            terrain_world.world_name.clone_from(&manifest.world_name);
        }
        for region in &manifest.regions {
            let handle: Handle<TerrainRegionAsset> = asset_server.load(region.path.clone());
            terrain_world.loaded_regions.insert(region.region(), handle);
        }
        info!(
            "Queued {} engine terrain region asset(s) from manifest '{}'",
            manifest.regions.len(),
            manifest.world_name
        );
    }
}
